#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use]
extern crate rocket;
#[macro_use]
extern crate serde_derive;

use rocket::Data;
use rocket::State;
use rocket::Outcome;
use v_htmlescape::escape;
use rocket_contrib::json::Json;
use rocket::response::NamedFile;
use rocket_contrib::serve::StaticFiles;
use rocket_contrib::templates::Template;
use rocket::request::{self, Form, FromRequest, Request};
use rocket::http::{Status, Cookie, Cookies, ContentType};

use std::str;
use reqwest::Client;
use clap::{App, Arg};
use chrono::DateTime;
use std::io::SeekFrom;
use std::error::Error;
use std::path::PathBuf;
use chrono::prelude::*;
use std::io::prelude::*;
use chrono::offset::Local;
use grep::printer::Standard;
use std::collections::HashMap;
use rocket::response::Redirect;
use grep::searcher::SearcherBuilder;
use grep::regex::RegexMatcherBuilder;
use std::fs::{self, File, OpenOptions};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, COOKIE};

use multipart::server::Multipart;
use multipart::server::save::SaveResult::*;

static mut GLOBAL_ARGS: Option<Args> = None;

macro_rules! log {
    ($($x: expr), +) => {
        let mut str = Local::now().to_rfc2822();
        str.push_str(": ");
        $(
            str = format!("{} {:?};", str, $x);
        )*
        println!("{}", str);
    };
}

#[derive(FromForm, Debug)]
struct Login {
    username: String,
    password: String,
}

#[derive(Debug, Clone)]
struct Args {
    file_dir: PathBuf,
    username: Option<String>,
    password: Option<String>,
    log: bool,
    write: bool,
}

#[derive(Debug, Serialize)]
struct IndexElement {
    class: String,
    name: String,
    date: String,
    size: u64,
}

impl IndexElement {
    fn new(class: String, name: String, date: String, size: u64) -> IndexElement {
        IndexElement {
            class,
            name,
            date,
            size,
        }
    }
}

#[derive(Debug, Serialize)]
struct IndexRender {
    status: bool,
    info: String,
    list: String,
    file_path: Option<String>,
}

impl IndexRender {
    fn new(
        status: bool,
        info: String,
        list: Vec<IndexElement>,
        file_path: Option<String>,
    ) -> IndexRender {
        let list_json = serde_json::to_string(&list).expect("error");
        IndexRender {
            status,
            info,
            list: list_json,
            file_path,
        }
    }
}

impl Args {
    fn new(file_dir: PathBuf, username: Option<String>, password: Option<String>, log: bool, write: bool) -> Args {
        Args {
            file_dir,
            username,
            password,
            log,
            write,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct DetailRender {
    content: String,
    seek: u64,
    file_path: String,
    write: bool,
}

impl DetailRender {
    fn new(content: String, file_path: String, seek: u64) -> DetailRender {
        DetailRender {
            content,
            seek,
            file_path,
            write: false,
        }
    }

    fn set_write(&mut self, write: bool) {
        self.write = write;
    }
}

#[derive(Debug, Serialize)]
struct ErrorRender {
    info: String,
}

impl ErrorRender {
    fn new(info: String) -> ErrorRender {
        ErrorRender { info }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SearchRender {
    search: String,
    content: String,
    file_path: String,
}

impl SearchRender {
    fn new(content: String, file_path: String, search: String) -> SearchRender {
        SearchRender {
            search,
            content,
            file_path,
        }
    }
}

struct Authorization;

#[derive(Debug)]
enum AuthorizationError {
    NoAuth,
}

impl<'a, 'r> FromRequest<'a, 'r> for Authorization {
    type Error = AuthorizationError;

    fn from_request(request: &'a Request<'r>) -> request::Outcome<Self, Self::Error> {
        let config = request.guard::<State<Args>>().unwrap();
        match &config.username {
            Some(_username) => {
                if !is_auth(&mut request.cookies(), &config) {
                    return Outcome::Failure((Status::Forbidden, AuthorizationError::NoAuth));
                }
            }
            _ => {}
        }

        Outcome::Success(Authorization)
    }
}

fn directory_filter(dir: String) -> String {
    unsafe {
        let base_dir = GLOBAL_ARGS.clone().unwrap().file_dir;
        dir.replace(base_dir.to_str().unwrap_or(""), "")
    }
}

// Gets a list of subfiles and directories in a directory
fn get_directory_info_render(path: &PathBuf) -> Result<IndexRender, Box<dyn Error>> {
    let render;
    if path.is_dir() {
        let mut elements: Vec<IndexElement> = vec![];

        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let file_path = entry.path();
            let metadata = entry.metadata()?;
            let file_name = file_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();

            if file_name.get(0..1) == Some(".") {
                continue;
            }

            let len = metadata.len();
            let atime: DateTime<Local> = metadata.modified()?.into();
            let atime_string = atime.format("%Y-%m-%d %T").to_string();

            let class = match file_path.is_dir() {
                true => "d".to_string(),
                false => "f".to_string(),
            };
            elements.push(IndexElement::new(class, file_name, atime_string, len));
        }
        render = IndexRender::new(
            true,
            "".to_string(),
            elements,
            Some(directory_filter(path.to_string_lossy().to_string())),
        );
    } else {
        render = IndexRender::new(false, "目录配置错误".to_string(), vec![], None);
    }

    Ok(render)
}

// Gets the content of a file 
fn get_detail_render(path: &PathBuf, start_seek: u64) -> Result<DetailRender, Box<dyn Error>> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    let file_len = metadata.len();
    let max_file_len = 512000;         // the max returned size, default is 512kb.
    let mut contents = String::new();
    let seek = file_len;
    
    if file_len > max_file_len || start_seek > 0 {
        let read_start_seek: u64;
        if start_seek > 0 {
            read_start_seek = start_seek;
        } else {
            read_start_seek = file_len - max_file_len;
        }

        let (c, _) = attemp_to_read_file(&mut file, read_start_seek, 3)?;
        contents = c;
    } else {
        file.read_to_string(&mut contents)?;
    }

    let render = DetailRender::new(contents, directory_filter(path.to_string_lossy().to_string()), seek);
    Ok(render)
}

// Try to read the file some times.
fn attemp_to_read_file(
    file: &mut File,
    seek: u64,
    times: u8,
) -> Result<(String, u64), Box<dyn Error>> {
    let mut buff = vec![];
    file.seek(SeekFrom::Start(seek))?;
    file.read_to_end(&mut buff)?;
    match String::from_utf8(buff) {
        Ok(s) => {
            return Ok((s, seek));
        }
        Err(e) => {
            if times > 0 {
                // That returned content that intercepted by Seek maybe is incomplete(multibyte encoding),so sets some offset 
                return attemp_to_read_file(file, seek + 1, times - 1);
            } else {
                return Err(Box::new(e));
            }
        }
    }
}

// Gets the filtered content of a file
fn get_search_render(
    path: &PathBuf,
    search: &str,
    before: &str,
    after: &str,
    case_insensitive: bool
) -> Result<SearchRender, Box<dyn Error>> {
    let single_page_limit = 10485760;   // The max size of filtered result per page
    let mut size_limit = 0; // The max read size of all pages
    let mut content = "".to_string();
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                let ret = get_search_render(&path, search, before, after, case_insensitive);
                if let Ok(render) = ret {
                    if render.content.len() > 0 {
                        size_limit = size_limit + single_page_limit;
                        content = format!(
                            "{}\r\n\r\n\r\n\r\n{}\r\n\r\n{}",
                            content,
                            directory_filter(path.to_str().unwrap().to_string()),
                            render.content
                        );
                    }
                }
            }
        }
    } else {
        let file = File::open(path)?;
        let mut matcher = RegexMatcherBuilder::new();
        matcher.case_insensitive(case_insensitive);
        let matcher = matcher.build(search)?;
        let mut search_build = SearcherBuilder::new();
        let mut printer = Standard::new_no_color(vec![]);
        let before_num: usize = before.parse()?;
        let after_num: usize = after.parse()?;
        size_limit = single_page_limit;
        search_build.multi_line(true);
        search_build.after_context(after_num);
        search_build.before_context(before_num);
        search_build
            .build()
            .search_file(&matcher, &file, printer.sink(&matcher))?;
        let search_bytes = printer.into_inner().into_inner();
        content = String::from_utf8(search_bytes)?;
    }

    if content.len() > size_limit {
        return Err("搜索结果太大，请使用更准确的搜索词")?;
    }

    Ok(SearchRender::new(
        content,
        path.to_string_lossy().to_string(),
        search.to_string(),
    ))
}

fn is_auth(cookies: &mut Cookies, config: &Args) -> bool {
    let config = config.to_owned().clone();
    let username = cookies
        .get_private("username")
        .map(|value| format!("{}", value));
    config.username.map(|value| format!("username={}", value)) == username
}

#[get("/")]
fn auth(args: State<Args>, mut cookies: Cookies) -> Redirect {
    if is_auth(&mut cookies, &*args) {
        Redirect::to("/file-reader-index")
    } else {
        Redirect::to("/login")
    }
}

#[get("/file-reader-index")]
fn index(args: State<Args>, _auth: Authorization) -> Template {
    match get_directory_info_render(&args.file_dir) {
        Ok(render) => {
            if args.log {
                log!("Access index");
            }
            return Template::render("index", render);
        }
        Err(e) => {
            let render = ErrorRender::new(e.to_string());
            return Template::render("error", render);
        }
    };
}

#[get("/login")]
fn login() -> Template {
    Template::render("login", ErrorRender::new("".to_owned()))
}

#[get("/debug")]
fn debug(args: State<Args>) -> Template {
    if args.log {
        log!("Access debug");
    }
    Template::render("debug", ErrorRender::new("".to_owned()))
}

#[post("/login", data = "<login>")]
fn do_login(args: State<Args>, login: Form<Login>, mut cookies: Cookies) -> String {
    let args = (*args).clone();
    let mut render: HashMap<String, String> = HashMap::new();
    if args.username.unwrap_or("".to_owned()) == login.username
        && args.password.unwrap_or("".to_owned()) == login.password
    {
        cookies.add_private(Cookie::new("username", login.username.clone()));
        render.insert("status".to_owned(), "1".to_owned());
    } else {
        render.insert("status".to_owned(), "0".to_owned());
        render.insert("msg".to_owned(), "帐号或密码错误".to_owned());
    }
    // login.username;
    serde_json::to_string(&render).unwrap_or(return_result(0, ""))
}

#[get("/more?<seek>&<path>", rank = 3)]
fn more(args: State<Args>, seek: u64, path: String, _auth: Authorization) -> String {
    let mut output = "".to_string();
    let path = &args.file_dir.join(path_to_relative(&PathBuf::from(path)));
    match get_detail_render(&path, seek) {
        Ok(render) => {
            if let Ok(a) = serde_json::to_string(&render) {
                output = a;
            }
        }
        Err(e) => {
            output = e.to_string();
        }
    }
    output
}

#[get("/search?<search>&<path>&<before>&<after>&<case_sensitive>", rank = 3)]
fn search(
    args: State<Args>,
    search: String,
    path: String,
    before: String,
    after: String,
    case_sensitive: bool,
    _auth: Authorization,
) -> Template {
    if args.log {
        log!(format!("Access search, path: {}, search: {}", path, search));
    }
    let case_insensitive = match case_sensitive {
        true => false,
        false => true
    };
    let path = &args.file_dir.join(path_to_relative(&PathBuf::from(path)));
    match get_search_render(&path, &search, &before, &after, case_insensitive) {
        Ok(render) => {
            return Template::render("search", render);
        }
        Err(e) => {
            let render = ErrorRender::new(e.to_string());
            return Template::render("error", render);
        }
    };
}

#[derive(Debug, Responder)]
enum DetailResponse {
    Template(Template),
    NamedFile(Option<NamedFile>),
}

#[get("/<name..>", rank = 100)]
fn detail(args: State<Args>, name: PathBuf, _auth: Authorization) -> DetailResponse {
    if args.log {
        log!(format!("Access detail, path:{}", name.to_string_lossy()));
    }
    let path = &args.file_dir.join(path_to_relative(&name));
    if path.is_dir() {
        match get_directory_info_render(&path) {
            Ok(render) => return DetailResponse::Template(Template::render("index", render)),
            Err(e) => {
                let render = ErrorRender::new(e.to_string());
                return DetailResponse::Template(Template::render("error", render));
            }
        };
    } else {
        match get_detail_render(&path, 0) {
            Ok(mut render) => {
                render.set_write(args.write);
                return DetailResponse::Template(Template::render("detail", render));
            },
            Err(_) => {
                // Download directly
                return DetailResponse::NamedFile(NamedFile::open(&path).ok());
            }
        }
    }
}

#[derive(Deserialize, Debug)]
struct DebugAgent {
    uri: String,
    json: String,
    cookie: String,  
    method: i8,
    urlencoded: String,
    enctype: String,
}

#[post("/debug_agent", data = "<params>")]
fn debug_agent(args: State<Args>, params: Json<DebugAgent>) -> String {
    if args.log {
        log!("Debug by agent");
    }
    let client = Client::new();
    let mut content = HashMap::new();
    let mut headers = HeaderMap::new();
    let mut data = params.json.clone();
    let content_type = match &params.enctype[..] {
        "json" => {
            "application/json"
        },
        "urlencoded" => {
            data = params.urlencoded.clone();
            "application/x-www-form-urlencoded"
        },
        _ => "application/json",
    };

    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(COOKIE, HeaderValue::from_static(string_to_static_str(params.cookie.clone())));

    let request_build = match params.method {
        1 => client.post(&(params.uri)),
        2 => client.get(&(params.uri)),
        _ => client.post(&(params.uri))
    };

    match request_build
        .headers(headers)
        .body(data)
        .send() {
        Ok(mut res) => {
            let mut body:String = "".to_string();
            res.read_to_string(&mut body).unwrap();

            content.insert("headers", format!("{:?}", res.headers()));
            content.insert("http_status", res.status().to_string());
            content.insert("status", "1".to_string());
            content.insert("message", "ok".to_string());
            content.insert("data", body);
        },
        Err(e) => {
            content.insert("status", "0".to_string());
            content.insert("message", e.to_string());
        },  
    }
        
    serde_json::to_string(&content).unwrap_or("{\"status\":\"0\"}".to_owned())
}

#[derive(Deserialize, Debug)]
struct AppendParams{
    path: PathBuf,
    content: String,
}

#[post("/append", data = "<params>")]
fn append(args: State<Args>, params: Json<AppendParams>) -> String {
    if args.log {
        log!("Append to file");
    }

    if !args.write {
        return return_result(0, "不支持写入");
    }

    let full_file_name = &args.file_dir.join(path_to_relative(&params.path));
    if let Ok(mut file) = OpenOptions::new()
        .write(true)
        .append(true)
        .open(&full_file_name) {
        if let Err(_e) = file.write_all(format!("\r\n{}", &escape(&params.content)).as_bytes()) {
            return return_result(0, "文件写入错误");
        }
    } else {
        return return_result(0, "文件打开错误");
    }
        
    return_result(1, "")
}

#[post("/upload?<path>&<file_name>", format="multipart/form-data", data = "<file>")]
fn upload(cont_type: &ContentType, args: State<Args>, file: Data, path: String, file_name: String) -> String {
    if args.log {
        log!("Upload to file");
    }

    if !args.write {
        return return_result(0, "不支持写入");
    }

    let file_path = &args.file_dir.join(path_to_relative(&PathBuf::from(path))).join(path_to_relative(&PathBuf::from(file_name)));

    let (_, boundary) = cont_type.params().find(|&(k, _)| k == "boundary").ok_or_else(
        || return_result(0, "格式错误")
    ).unwrap();

    match Multipart::with_body(file.open(), boundary).read_entry().unwrap().unwrap()
        .data.save().memory_threshold(0).with_path(file_path) {
        Full(_entries) => (println!("{:?}", _entries)),
        Partial(_partial, e) => {
            if args.log {
                log!("Upload error: {:?}", e);
            }
            return return_result(0, "写入错误");
        },
        Error(e) => {
            if args.log {
                log!("Upload error: {:?}", e);
            }
            return return_result(0, "写入错误");
        },
    }
    
    return_result(1, "")
}

#[post("/file_exist?<path>&<file_name>")]
fn file_exist(args: State<Args>, path: String, file_name: String) -> String {
    if args.log {
        log!("Append to file");
    }

    if !args.write {
        return return_result(0, "不支持写入");
    }

    let file_path = &args.file_dir.join(path_to_relative(&PathBuf::from(path))).join(path_to_relative(&PathBuf::from(file_name)));
    if file_path.as_path().exists() {
        return return_result(1, "");
    } else {
        return return_result(0, "");
    }
}

fn string_to_static_str(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn return_result(status: u8, msg: &str) -> String {
    return format!("{{\"status\":{}, \"message\":\"{}\"}}", status, msg);
}

fn path_to_relative(path: &PathBuf) -> PathBuf {
    let mut new_path = path.to_string_lossy().to_string();
    if path.is_absolute() {
        new_path = ".".to_string() + &new_path;
    }
    PathBuf::from(new_path.replace("..", ""))
}

fn main() {
    let args: Args = parse_arguments();
    unsafe {
        GLOBAL_ARGS = Some(args.clone());
    }
    let app = rocket::ignite()
        .manage(args)
        .mount(
            "/",
            routes![auth, index, detail, more, search, login, do_login, debug, debug_agent, append, upload, file_exist],
        )
        .mount("/public", StaticFiles::from("./templates/static"))
        .attach(Template::fairing());
    app.launch();
}

fn parse_arguments() -> Args {
    let matches = App::new("file_reader")
        .version("1.0")
        .author("smoothsea")
        .arg(
            Arg::with_name("directory")
                .short("d")
                .long("directory")
                .help("查看文件的目录")
                .required(true)
                .takes_value(true),
        )
        .arg(
            Arg::with_name("username")
                .short("u")
                .long("username")
                .help("验证登录用户名")
                .requires("password")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("password")
                .short("p")
                .long("password")
                .help("验证用户密码")
                .requires("username")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("log")
            .short("l")
            .help("输出日志")
            .takes_value(false),
        )
        .arg(
            Arg::with_name("write")
            .short("w")
            .help("写入文件")
            .takes_value(false),
        )
        .get_matches();

    let dir = PathBuf::from(matches.value_of("directory").unwrap());

    let username = match matches.is_present("username") {
        true => Some(matches.value_of("username").unwrap().to_owned()),
        false => None,
    };

    let password = match matches.is_present("password") {
        true => Some(matches.value_of("password").unwrap().to_owned()),
        false => None,
    };

    let log = matches.is_present("log");
    let write = matches.is_present("write");
    Args::new(dir, username, password, log, write)
}
