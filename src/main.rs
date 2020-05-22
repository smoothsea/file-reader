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
use std::error::Error;

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
    fn new(class: String, name: String, date: String) -> IndexElement {
        IndexElement { class, name, date }
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
        IndexRender { status, info, list }
    }
}

impl Args {
    fn new(file_dir: String) -> Args {
        Args { file_dir: file_dir }
    }
}

#[derive(Debug, Serialize)]
struct DetailRender {
    content: String,    
}

impl DetailRender {
    fn new(content: String) -> DetailRender {
        DetailRender {
            content
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


fn get_directory_info_render(dir: &str) -> Result<IndexRender, Box<dyn Error>> {
    let path = Path::new(dir);
    let mut render = IndexRender::new(false, "目录配置错误".to_string(), vec![]);
    if (path.is_dir()) {
        let mut elements: Vec<IndexElement> = vec![];

        for entry in fs::read_dir(path).expect("xx") {
            let file_path = entry.expect("ff").path();
            let mut file_name = file_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();

            let mut class = "".to_string();
            if (file_path.is_dir()) {
                class = "d".to_string();
            } else {
                class = "f".to_string();
            }
            
            elements.push(IndexElement::new(class, file_name, "x".to_string()));
        }
        render = IndexRender::new(true, "".to_string(), elements);
    } else {
        render = IndexRender::new(false, "目录配置错误".to_string(), vec![]);
    }

    Ok(render)
}

fn get_detail_render(file: &str) -> Result<DetailRender, Box<dyn Error>> {
    let path = Path::new(file);
    let mut render = DetailRender::new("".to_string());

    Ok(render)
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

#[get("/<name..>")]
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
        let file_result = File::open(full_path);
        let mut file;
        match file_result {
            Ok(f) => {
                file = f; 
            },
            Err(e) => {
                return Template::render("error", ErrorRender::new(e.to_string()));
            }
        }

        let mut contents = String::new();
        if let Err(e) = file.read_to_string(&mut contents) {
            return Template::render("error", ErrorRender::new(e.to_string()));
        }

        let render = DetailRender::new(contents);
        Template::render("detail", render)
    }
}

fn main() {
    let args: Args = parse_arguments();
    let app = rocket::ignite()
        .manage(args)
        .mount("/", routes![index, detail])
        .attach(Template::fairing());
    app.launch();
}

fn parse_arguments() -> Args {
    let args: Vec<String> = env::args().collect();

    if (args.len() <= 1) {
        panic!("Arguments can't be empty");
    }
    let file_dir: String = (&args[1]).to_string();
    Args::new(file_dir)
}
