#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use]
extern crate rocket;
#[macro_use]
extern crate serde_derive;

use rocket::State;
use rocket_contrib::templates::Template;
use std::env;
use std::path::Path;
use std::collections::HashMap;
use std::fs::{self, DirEntry};
use std::io;

#[derive(Debug)]
struct Args {
    file_dir: String,
}

#[derive(Debug, Serialize)]
struct IndexElement {
    class: String,
    name: String,
    date: String,
}

impl IndexElement {
    fn new(class:String, name:String, date:String) -> IndexElement {
        IndexElement {
            class,
            name,
            date,
        }
    }
}

#[derive(Debug, Serialize)]
struct IndexRender {
    status: bool,
    info: String,
    list: Vec<IndexElement>,
}

impl IndexRender {
    fn new(status: bool, info: String, list: Vec<IndexElement>) -> IndexRender {
        IndexRender {
            status,
            info,
            list,
        }
    }
}

impl Args {
    fn new(file_dir: String) -> Args {
        Args { file_dir: file_dir }
    }
}

#[get("/")]
fn index(args: State<Args>) -> Template {
    let path = Path::new(&args.file_dir);

    let mut render = IndexRender::new(false, "目录配置错误".to_string(), vec!());
    if (path.is_dir()) {
        for entry in fs::read_dir(path).expect("错误") {
            let file_path = entry.expect("文件错误").path();
            let mut file_name = "".to_string();
            if let Some(name) = file_path.file_name() {
                file_name = name.to_str();
            }

            println!("{:?}", file_name);

            let mut class = "".to_string(); 
            if (file_path.is_dir()) {
                class = "dir".to_string();
            } else {
                class = "file".to_string();
            }

            
        }
    } else {
        render = IndexRender::new(false, "目录配置错误".to_string(), vec!());
    }

    Template::render("index", render)
}

fn main() {
    let args: Args = parse_arguments();
    let dir = rocket::ignite()
        .manage(args)
        .mount("/", routes![index])
        .attach(Template::fairing())
        .launch();
}

fn parse_arguments() -> Args {
    let args: Vec<String> = env::args().collect();

    if (args.len() <= 1) {
        panic!("Arguments can't be empty");
    }
    let file_dir: String = (&args[1]).to_string();
    Args::new(file_dir)
}
