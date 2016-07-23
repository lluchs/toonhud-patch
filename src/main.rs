#[macro_use] extern crate lazy_static;
extern crate rquery;
extern crate itertools;
extern crate regex;

use std::io::{self, Read, Write, ErrorKind};
use std::fs::{self, File};
use regex::Regex;
use itertools::Itertools;
use rquery::{Document, Element};

fn main() {
    // TODO: Download these directly.
    let overrides = Document::new_from_xml_file("overrides.xml").unwrap();
    let theme = Document::new_from_xml_file("theme.xml").unwrap();

    let fields = theme.select_all("field").unwrap();
    //println!("have {} fields", fields.count());
    for field in fields {
        let id = field.attr("id").unwrap();
        let ftype = field.attr("type").unwrap();
        let valuetype = field.attr("valuetype");
        let value = field.select("value").unwrap().text();

        let id = if ftype == "radio" && valuetype.is_none() { id.to_string() + value } else { id.to_string() };
        println!("field: {} => {}", id, value);

        let actions = overrides.select_all(&format!("override[id={}] action", id)).unwrap();
        for action in actions {
            apply_action(&id, &action, value);
        }
    }
}

fn apply_action(id: &str, action: &Element, value: &str) {
    // TODO: Check requires
    let atype = action.attr("type").unwrap();
    println!("  action {}", atype);
    match atype.as_ref() {
        "replace" => { // Replace a file
            let old = get_text(action, "old").unwrap();
            let new = get_text(action, "new").unwrap();
            copy_file(new, old).unwrap();
            println!("    Replaced file {} with {}", old, new);
        },
        "color" => {
            let file = match action.select("file") {
                Ok(el) => el.text(),
                Err(_) => "toonhud/resource/clientscheme_colors.res"
            };
            let comment = get_text(action, "comment").unwrap();
            change_color(file, comment, value).unwrap();
            let adj: Option<(&str, i32)> = match id {
                "colorMenuText"       => Some(("Dimm", 120)),
                "colorFooter"         => Some(("Dimm", 200)),
                "colorFooterText"     => Some(("Dimm", 120)),
                "alphacolorInputText" => Some(("Dimm", 100)),
                "alphacolorPanelBg"   => Some(("Opaque", 255)),
                _ => if id.starts_with("colorQuality") || id.starts_with("colorRarity") { Some(("Dimm", 100)) } else { None }
            };
            if let Some((what, shift)) = adj {
                change_color(file, &format!("{} {}", what, comment), &change_opacity(value, shift)).unwrap();
            }
        },
        "remove" => { // Remove a file
            let path = get_text(action, "path").unwrap();
            remove_file(path).unwrap();
            println!("    Removed file {}", path);
        },
        "removefiles" => { // Remove multiple files separated by |
            for path in value.split('|') {
                remove_file(path).unwrap();
                println!("    Removed file {}", path);
            }
        },
        "edit" => { // Edit files with a crazy regex
            lazy_static! {
                static ref RE: Regex = Regex::new(r#"([\t ]+"[a-zA-Z_ \$]+"[\t ]+")([-+0-9a-zA-Z.%_ /\\\\]*)("[\t ]*[!\[$A-Z/\]]*[\t ]*[//]*[\t ]*[0-9A-Za-z ]*)"#).unwrap();
            }
            let path = get_text(action, "file").unwrap();
            let comment = get_text(action, "comment").unwrap();
            let prevalue = get_text(action, "prevalue").unwrap_or("");
            let afvalue = get_text(action, "afvalue").unwrap_or("");
            let value = &match get_text(action, "value").unwrap() {
                "input" => format!("{}{}{}", prevalue, value, afvalue),
                other => other.to_string()
            };
            // TODO: chatTextAntialias special case?
            let result = read_file(path).unwrap()
                .lines()
                .map(|line| {
                    if line.find(comment).is_some() {
                        RE.replace(line, |caps: &regex::Captures| format!("{}{}{}", caps.at(1).unwrap_or(""), value, caps.at(3).unwrap_or("")))
                    } else {
                        line.to_string()
                    }
                })
                .join("\n");
            write_file(path, &result).unwrap();
            println!("    Edit: {} = {} in {}", comment, value, path);
        },
        "removeline" => { // Remove lines identified by a comment
            let path = get_text(action, "file").unwrap();
            let comment = format!("// {}", get_text(action, "file").unwrap());
            let file = read_file(path).unwrap();
            let mut result = String::new();
            let mut count = 0;
            for line in file.lines() {
                if line.ends_with(&comment) {
                    count += 1;
                } else {
                    result.push_str(line);
                    result.push('\n');
                }
            }
            write_file(path, &result).unwrap();
            println!("    Removed {} lines with comment {} in {}", count, comment, path);
        },
        "animationlength" => {
            println!("    animationlength not implemented");
        },
        "replaceword" => {
            let path = get_text(action, "file").unwrap();
            let comment = get_text(action, "comment").unwrap();
            let prevalue = get_text(action, "prevalue").unwrap_or("");
            let afvalue = get_text(action, "afvalue").unwrap_or("");
            let old = get_text(action, "old").unwrap();
            let new = &match get_text(action, "new").unwrap() {
                "input" => format!("{}{}{}", prevalue, value, afvalue),
                other => other.to_string()
            };
            let result = read_file(path).unwrap()
                .lines()
                .map(|line| {
                    if line.find(comment).is_some() {
                        line.replace(old, new)
                    } else {
                        line.to_string()
                    }
                })
                .join("\n");
            write_file(path, &result).unwrap();
            println!("    Replaced '{}' with '{}' where comment '{}' was found in {}", old, new, comment, path);
        },
        _ => panic!("invalid action {}", atype)
    }
}

fn get_text<'a>(el: &'a Element, tag_name: &str) -> Option<&'a str> {
    match el.select(tag_name) {
        Ok(tag) => Some(tag.text()),
        Err(_)  => None
    }
}

fn change_opacity(color: &str, shift: i32) -> String {
    lazy_static! {
        static ref RE: Regex = Regex::new(r#" [0-9]*$"#).unwrap();
    }
    RE.replace_all(color, regex::NoExpand(&format!(" {}", shift)))
}

fn change_color(path: &str, comment: &str, to: &str) -> io::Result<()> {
    let re = Regex::new(&format!(r#""([-a-zA-Z0-9_ ]*)"( *)//( *){}"#, comment)).unwrap();
    let file = try!(read_file(path));
    let result = re.replace_all(&file, regex::NoExpand(&format!(r#""{}" // {}"#, to, comment)));
    write_file(path, &result)
}

// Panics if the path points to anything other than a toonhud subdirectory.
fn verify_path(path: &str) {
    if !path.starts_with("toonhud/") || path.find("..").is_some() {
        panic!("Unsafe path '{}'", path);
    }
}

fn read_file(filename: &str) -> io::Result<String> {
    verify_path(filename);
    let mut file = try!(File::open(filename));
    let mut result = String::new();
    try!(file.read_to_string(&mut result));
    Ok(result)
}

fn write_file(filename: &str, contents: &str) -> io::Result<()> {
    verify_path(filename);
    let mut file = try!(File::create(filename));
    try!(file.write(contents.as_bytes()));
    Ok(())
}

fn remove_file(path: &str) -> io::Result<()> {
    verify_path(path);
    match fs::remove_file(path) {
        Ok(_) => Ok(()),
        Err(err) => {
            match err.kind() {
                ErrorKind::NotFound => Ok(()), // file to remove is not there
                _ => Err(err) // real error
            }
        }
    }
}

fn copy_file(from: &str, to: &str) -> io::Result<u64> {
    verify_path(from);
    verify_path(to);
    fs::copy(from, to)
}
