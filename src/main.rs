#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use]
extern crate rocket;
#[macro_use]
extern crate serde_derive;

use rocket::State;
use rocket_contrib::templates::Template;
use std::env;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::io::prelude::*;
use std::io::{SeekFrom};
use std::str;
use std::error::Error;
use serde::{Serialize, Deserialize};
use chrono::offset::Utc;
use chrono::DateTime;

#[derive(Debug)]
struct Args {
    file_dir: String,
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
        IndexElement { class, name, date, size }
    }
}

#[derive(Debug, Serialize)]
struct IndexRender {
    status: bool,
    info: String,
    list: String,
}

impl IndexRender {
    fn new(status: bool, info: String, list: Vec<IndexElement>) -> IndexRender {
        let list_json = serde_json::to_string(& list).expect("error");
        IndexRender { status, info, list:list_json}
    }
}

impl Args {
    fn new(file_dir: String) -> Args {
        Args { file_dir: file_dir }
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
            file_path
        }
    }
}

#[derive(Debug, Serialize)]
struct ErrorRender {
    info: String,
}

impl ErrorRender {
    fn new(info: String) -> ErrorRender {
        ErrorRender {
            info
        }
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

fn get_directory_info_render(dir: &str) -> Result<IndexRender, Box<dyn Error>> {
    let path = Path::new(dir);
    let mut render = IndexRender::new(false, "目录配置错误".to_string(), vec![]);
    if (path.is_dir()) {
        let mut elements: Vec<IndexElement> = vec![];

        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let file_path = entry.path();
            let metadata = entry.metadata()?;
            let mut file_name = file_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();

            if (file_name.get(0..1) == Some(".")) {
                continue;
            }

            let len = metadata.len();
            let atime: DateTime<Utc> = metadata.modified()?.into();
            let atime_string = atime.format("%Y-%m-%d %T").to_string();

            let mut class = "".to_string();
            if (file_path.is_dir()) {
                class = "d".to_string();
            } else {
                class = "f".to_string();
            }
            
            elements.push(IndexElement::new(class, file_name, atime_string, len));
        }
        render = IndexRender::new(true, "".to_string(), elements);
    } else {
        render = IndexRender::new(false, "目录配置错误".to_string(), vec![]);
    }

    Ok(render)
}

fn get_detail_render(file_path: &str, start_seek: u64) -> Result<DetailRender, Box<dyn Error>> {
    let path = Path::new(file_path);
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    let file_len = metadata.len();
    let max_file_len = 5181440;
    
    let mut contents = String::new();
    let mut seek = file_len;
    if (file_len > max_file_len || start_seek > 0) {
        let len_if_exceed = 512000; //500kb
        let mut read_start_seek:u64;
        if (start_seek > 0) {
            read_start_seek = start_seek;
        } else {
            read_start_seek = file_len - len_if_exceed;
        }
        if let (a, b) = attemp_to_read_file(&mut file, read_start_seek, 3)? {
            contents = a;
        }
    } else {
        file.read_to_string(&mut contents)?;
    }

    let render = DetailRender::new(contents, file_path.to_string(), seek);
    Ok(render)
}

fn attemp_to_read_file(file:&mut File, seek:u64, times:u8) -> Result<(String, u64), Box<dyn Error>> {
    let mut content = "".to_string();
    let mut buff = vec![];
    file.seek(SeekFrom::Start(seek));
    file.read_to_end(&mut buff);
    match String::from_utf8(buff) {
        Ok(s) => {
            return Ok((s, seek));
        },
        Err(e) => {
            if (times > 0) {
                return attemp_to_read_file(file, seek + 1, times - 1)
            } else {
                return Err(Box::new(e));
            }
        }
    }

    Ok((content, seek))
}

fn get_search_render(file_path: &str, search: &str) -> Result<SearchRender, Box<dyn Error>> {
    let path = Path::new(file_path);
    let mut file = File::open(path)?;
    
    
}


#[get("/")]
fn index(args: State<Args>) -> Template {
    match get_directory_info_render(&args.file_dir) {
        Ok(render) => {
            return Template::render("index", render);
        },
        Err(e) => {
            let render = ErrorRender::new(e.to_string());
            return Template::render("error", render);      
        } 
    };
}


#[get("/more?<seek>&<path>", rank = 3)]
fn more(args: State<Args>, seek:u64, path:String) -> String {
    let mut output = "".to_string();
    match get_detail_render(&path, seek) {
        Ok(render) => {
           if let Ok(a) =  serde_json::to_string(& render) {
               output = a;
           }
        },
        Err(e) => {
            output = e.to_string();
        }
    }
    output
}

#[get("/search?<search>&<path>", rank = 3)]
fn search(args: State<Args>, search:String, path:String) -> Template {
    match get_search_render(&path, &search) {
        Ok(render) => {
            return Template::render("index", render);
        },
        Err(e) => {
            let render = ErrorRender::new(e.to_string());
            return Template::render("error", render);      
        } 
    };
}


#[get("/<name..>", rank = 4)]
fn detail(args: State<Args>, name: PathBuf) -> Template {
    let path = &args.file_dir;
    let full_file_name = format!("{}{}", path, name.to_string_lossy().into_owned());
    let full_path = Path::new(&full_file_name);
    if (full_path.is_dir()) {
        match get_directory_info_render(&full_file_name) {
            Ok(render) => {
                return Template::render("index", render)
            },
            Err(e) => {
                let render = ErrorRender::new(e.to_string());
                return Template::render("error", render);      
            } 
        };
    } else {
        match get_detail_render(&full_file_name, 0) {
            Ok(render) => {
                return Template::render("detail", render);
            },
            Err(e) => {
                let render = ErrorRender::new(e.to_string());
                return Template::render("error", render);
            }
        }
    }
}

fn main() {
    let args: Args = parse_arguments();
    let app = rocket::ignite()
        .manage(args)
        .mount("/", routes![index, detail, more])
        .attach(Template::fairing());
    app.launch();
}

fn parse_arguments() -> Args {
    let args: Vec<String> = env::args().collect();

    if (args.len() <= 1) {
        panic!("Arguments can't be empty");
    }
    let mut file_dir: String = (&args[1]).to_string();
    if let Some(o) = file_dir.pop() {
        if (o.to_string() != "/".to_string()) {
            println!("ok");
            file_dir = format!("{}{}", file_dir, o.to_string());
        }
        file_dir = format!("{}{}", file_dir, "/".to_string());
    };
    Args::new(file_dir)
}
