#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use]
extern crate rocket;
#[macro_use]
extern crate serde_derive;

use chrono::offset::Utc;
use chrono::DateTime;
use clap::{App, Arg};
use grep::printer::Standard;
use grep::regex::RegexMatcher;
use grep::searcher::SearcherBuilder;
use rocket::http::Status;
use rocket::http::{Cookie, Cookies};
use rocket::request::{self, Form, FromRequest, Request};
use rocket::response::Redirect;
use rocket::Outcome;
use rocket::State;
use rocket_contrib::templates::Template;
use std::collections::HashMap;
use std::error::Error;
use std::fs::{self, File};
use std::io::prelude::*;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::str;

static mut GLOBAL_ARGS: Option<Args> = None;

#[derive(FromForm, Debug)]
struct Login {
    username: String,
    password: String,
}

#[derive(Debug, Clone)]
struct Args {
    file_dir: String,
    username: Option<String>,
    password: Option<String>,
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
    fn new(file_dir: String, username: Option<String>, password: Option<String>) -> Args {
        Args {
            file_dir,
            username,
            password,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct DetailRender {
    content: String,
    seek: u64,
    file_path: String,
}

impl DetailRender {
    fn new(content: String, file_path: String, seek: u64) -> DetailRender {
        DetailRender {
            content,
            seek,
            file_path,
        }
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

fn get_complete_directory(dir: &str) -> String {
    unsafe {
        let base_dir = GLOBAL_ARGS.clone().unwrap().file_dir;
        if dir.contains(&base_dir) {
            dir.to_owned()
        } else {
            format!("{}{}", base_dir, dir)
        }
    }
}

fn directory_filter(dir: String) -> String {
    unsafe {
        let base_dir = GLOBAL_ARGS.clone().unwrap().file_dir;
        dir.replace(&base_dir, "")
    }
}

fn get_directory_info_render(dir: &str) -> Result<IndexRender, Box<dyn Error>> {
    let dir = &get_complete_directory(dir);
    let path = Path::new(dir);
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
                .into_owned();

            if file_name.get(0..1) == Some(".") {
                continue;
            }

            let len = metadata.len();
            let atime: DateTime<Utc> = metadata.modified()?.into();
            let atime_string = atime.format("%Y-%m-%d %T").to_string();

            let class;
            if file_path.is_dir() {
                class = "d".to_string();
            } else {
                class = "f".to_string();
            }
            elements.push(IndexElement::new(class, file_name, atime_string, len));
        }
        render = IndexRender::new(
            true,
            "".to_string(),
            elements,
            Some(directory_filter(path.to_str().unwrap_or("").to_string())),
        );
    } else {
        render = IndexRender::new(false, "目录配置错误".to_string(), vec![], None);
    }

    Ok(render)
}

fn get_detail_render(file_path: &str, start_seek: u64) -> Result<DetailRender, Box<dyn Error>> {
    let file_path = &get_complete_directory(file_path);
    let path = Path::new(file_path);
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    let file_len = metadata.len();
    let max_file_len = 5181440;
    let mut contents = String::new();
    let seek = file_len;
    if file_len > max_file_len || start_seek > 0 {
        let len_if_exceed = 512000; //500kb
        let read_start_seek: u64;
        if start_seek > 0 {
            read_start_seek = start_seek;
        } else {
            read_start_seek = file_len - len_if_exceed;
        }
        if let (a, _b) = attemp_to_read_file(&mut file, read_start_seek, 3)? {
            contents = a;
        }
    } else {
        file.read_to_string(&mut contents)?;
    }

    let render = DetailRender::new(contents, directory_filter(file_path.to_string()), seek);
    Ok(render)
}

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
                return attemp_to_read_file(file, seek + 1, times - 1);
            } else {
                return Err(Box::new(e));
            }
        }
    }
}

fn get_search_render(
    file_path: &str,
    search: &str,
    before: &str,
    after: &str,
) -> Result<SearchRender, Box<dyn Error>> {
    let file_path = &get_complete_directory(file_path);
    let path = Path::new(file_path);
    let single_page_limit = 10485760;
    let mut size_limit = 0;
    let mut content = "".to_string();
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                let ret = get_search_render(path.to_str().unwrap(), search, before, after);
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
        let matcher = RegexMatcher::new(search)?;
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
        file_path.to_string(),
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
        Redirect::to("/index")
    } else {
        Redirect::to("/login")
    }
}

#[get("/index")]
fn index(args: State<Args>, _auth: Authorization) -> Template {
    match get_directory_info_render(&args.file_dir) {
        Ok(render) => {
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
    serde_json::to_string(&render).unwrap_or("{\"status\":\"0\"}".to_owned())
}

#[get("/more?<seek>&<path>", rank = 3)]
fn more(seek: u64, path: String, _auth: Authorization) -> String {
    let mut output = "".to_string();
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

#[get("/search?<search>&<path>&<before>&<after>", rank = 3)]
fn search(
    _args: State<Args>,
    search: String,
    path: String,
    before: String,
    after: String,
    _auth: Authorization,
) -> Template {
    match get_search_render(&path, &search, &before, &after) {
        Ok(render) => {
            return Template::render("search", render);
        }
        Err(e) => {
            let render = ErrorRender::new(e.to_string());
            return Template::render("error", render);
        }
    };
}

#[get("/<name..>", rank = 4)]
fn detail(args: State<Args>, name: PathBuf, _auth: Authorization) -> Template {
    let path = &args.file_dir;
    let full_file_name = format!("{}{}", path, name.to_string_lossy().into_owned());
    let full_path = Path::new(&full_file_name);
    if full_path.is_dir() {
        match get_directory_info_render(&full_file_name) {
            Ok(render) => return Template::render("index", render),
            Err(e) => {
                let render = ErrorRender::new(e.to_string());
                return Template::render("error", render);
            }
        };
    } else {
        match get_detail_render(&full_file_name, 0) {
            Ok(render) => {
                return Template::render("detail", render);
            }
            Err(e) => {
                let render = ErrorRender::new(e.to_string());
                return Template::render("error", render);
            }
        }
    }
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
            routes![auth, index, detail, more, search, login, do_login],
        )
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
        .get_matches();

    let mut dir = matches.value_of("directory").unwrap().to_owned();
    if !dir.ends_with("/") {
        dir.push_str("/");
    }

    let username = match matches.is_present("username") {
        true => Some(matches.value_of("username").unwrap().to_owned()),
        false => None,
    };
    let password = match matches.is_present("password") {
        true => Some(matches.value_of("password").unwrap().to_owned()),
        false => None,
    };
    Args::new(dir, username, password)
}
