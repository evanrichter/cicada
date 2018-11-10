use std::env;
use std::error::Error;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::io::IntoRawFd;
use std::path::Path;

use regex::Regex;
use time;

use execute;
use libc;
use parsers;
use shell;

macro_rules! println_stderr {
    ($fmt:expr) => (
        match writeln!(&mut ::std::io::stderr(), $fmt) {
            Ok(_) => {}
            Err(e) => println!("write to stderr failed: {:?}", e)
        }
    );
    ($fmt:expr, $($arg:tt)*) => (
        match writeln!(&mut ::std::io::stderr(), $fmt, $($arg)*) {
            Ok(_) => {}
            Err(e) => println!("write to stderr failed: {:?}", e)
        }
    );
}

pub fn clog(s: &str) {
    let file;
    if let Ok(x) = env::var("CICADA_LOG_FILE") {
        file = x;
    } else {
        return;
    }
    let mut cfile;
    match OpenOptions::new().append(true).create(true).open(&file) {
        Ok(x) => cfile = x,
        Err(e) => {
            println!("clog: open file {} failed: {:?}", &file, e);
            return;
        }
    }
    let pid = unsafe { libc::getpid() };
    let now = time::now();
    let s = format!(
        "[{:04}-{:02}-{:02} {:02}:{:02}:{:02}][{}]{}",
        now.tm_year + 1900,
        now.tm_mon + 1,
        now.tm_mday,
        now.tm_hour,
        now.tm_min,
        now.tm_sec,
        pid,
        s,
    );
    let s = if s.ends_with('\n') {
        s
    } else {
        format!("{}\n", s)
    };
    match cfile.write_all(s.as_bytes()) {
        Ok(_) => {}
        Err(e) => {
            println!("clog: write_all failed: {:?}", e);
            return;
        }
    }
}

macro_rules! log {
    ($fmt:expr) => (
        clog($fmt);
    );
    ($fmt:expr, $($arg:tt)*) => (
        clog(&format!($fmt, $($arg)*));
    );
}

pub fn get_user_name() -> String {
    match env::var("USER") {
        Ok(x) => {
            return x;
        }
        Err(e) => {
            log!("cicada: env USER error: {:?}", e);
        }
    }
    let cmd_result = execute::run("whoami");
    return cmd_result.stdout.trim().to_string();
}

pub fn get_user_home() -> String {
    match env::var("HOME") {
        Ok(x) => x,
        Err(e) => {
            println!("cicada: env HOME error: {:?}", e);
            String::new()
        }
    }
}

pub fn get_user_completer_dir() -> String {
    let home = get_user_home();
    format!("{}/.cicada/completers", home)
}

pub fn get_rc_file() -> String {
    let home = get_user_home();
    format!("{}/{}", home, ".cicadarc")
}

pub fn unquote(s: &str) -> String {
    let args = parsers::parser_line::line_to_plain_tokens(s);
    if args.is_empty() {
        return String::new();
    }
    args[0].clone()
}

pub fn is_export_env(line: &str) -> bool {
    re_contains(line, r"^ *export +[a-zA-Z0-9_]+=.*$")
}

pub fn is_env(line: &str) -> bool {
    re_contains(line, r"^[a-zA-Z0-9_]+=.*$")
}

pub fn should_extend_brace(line: &str) -> bool {
    re_contains(line, r#"\{[^ "']+,[^ "']+,?[^ "']*\}"#)
}

// #[allow(clippy::trivial_regex)]
pub fn extend_bandband(sh: &shell::Shell, line: &mut String) {
    if !re_contains(line, r"!!") {
        return;
    }
    if sh.previous_cmd.is_empty() {
        return;
    }

    let re;
    match Regex::new(r"!!") {
        Ok(x) => {
            re = x;
        }
        Err(e) => {
            println_stderr!("Regex new: {:?}", e);
            return;
        }
    }

    let mut replaced = false;
    let mut new_line = String::new();
    let tokens = parsers::parser_line::cmd_to_tokens(line);
    for (sep, token) in tokens {
        if !sep.is_empty() {
            new_line.push_str(&sep);
        }

        if re_contains(&token, r"!!") && sep != "'" {
            let line2 = token.clone();
            let result = re.replace_all(&line2, sh.previous_cmd.as_str());
            new_line.push_str(&result);
            replaced = true;
        } else {
            new_line.push_str(&token);
        }

        if !sep.is_empty() {
            new_line.push_str(&sep);
        }
        new_line.push(' ');
    }

    *line = new_line.trim_right().to_string();
    // print full line after extending
    if replaced {
        println!("{}", line);
    }
}

pub fn wrap_sep_string(sep: &str, s: &str) -> String {
    let mut _token = String::new();
    let mut met_subsep = false;
    // let set previous_subsep to any char except '`' or '"'
    let mut previous_subsep = 'N';
    for c in s.chars() {
        // handle cmds like: export DIR=`brew --prefix openssl`/include
        // or like: export foo="hello world"
        if sep.is_empty() && (c == '`' || c == '"') {
            if !met_subsep {
                met_subsep = true;
                previous_subsep = c;
            } else if c == previous_subsep {
                met_subsep = false;
                previous_subsep = 'N';
            }
        }
        if c.to_string() == sep {
            _token.push('\\');
        }
        if c == ' ' && sep.is_empty() && !met_subsep {
            _token.push('\\');
        }
        _token.push(c);
    }
    format!("{}{}{}", sep, _token, sep)
}

pub fn env_args_to_command_line() -> String {
    let mut result = String::new();
    let env_args = env::args();
    if env_args.len() <= 1 {
        return result;
    }
    for (i, arg) in env_args.enumerate() {
        if i == 0 || arg == "-c" {
            continue;
        }
        result.push_str(arg.as_str());
    }
    result
}

pub fn is_alias(line: &str) -> bool {
    re_contains(line, r"^ *alias +[a-zA-Z0-9_\.-]+=.*$")
}

extern "C" {
    fn gethostname(name: *mut libc::c_char, size: libc::size_t) -> libc::c_int;
}

/// via: https://gist.github.com/conradkleinespel/6c8174aee28fa22bfe26
pub fn get_hostname() -> String {
    let len = 255;
    let mut buf = Vec::<u8>::with_capacity(len);

    let ptr = buf.as_mut_slice().as_mut_ptr();

    let err = unsafe { gethostname(ptr as *mut libc::c_char, len as libc::size_t) } as i32;

    match err {
        0 => {
            let real_len;
            let mut i = 0;
            loop {
                let byte = unsafe { *(((ptr as u64) + (i as u64)) as *const u8) };
                if byte == 0 {
                    real_len = i;
                    break;
                }

                i += 1;
            }
            unsafe { buf.set_len(real_len) }
            String::from_utf8_lossy(buf.as_slice()).into_owned()
        }
        _ => String::from("unknown"),
    }
}

pub fn is_arithmetic(line: &str) -> bool {
    if !re_contains(line, r"[0-9]+") {
        return false;
    }
    re_contains(line, r"^[ 0-9\.\(\)\+\-\*/]+$")
}

pub fn re_contains(line: &str, ptn: &str) -> bool {
    let re;
    match Regex::new(ptn) {
        Ok(x) => {
            re = x;
        }
        Err(e) => {
            println!("Regex new: {:?}", e);
            return false;
        }
    }
    re.is_match(line)
}

pub fn create_raw_fd_from_file(file_name: &str, append: bool) -> Result<i32, String> {
    let mut oos = OpenOptions::new();
    if append {
        oos.append(true);
    } else {
        oos.write(true);
        oos.truncate(true);
    }
    match oos.create(true).open(file_name) {
        Ok(x) => {
            let fd = x.into_raw_fd();
            Ok(fd)
        }
        Err(e) => Err(format!("failed to create fd from file: {:?}", e)),
    }
}

pub fn get_fd_from_file(file_name: &str) -> i32 {
    let path = Path::new(file_name);
    let display = path.display();
    let file = match File::open(&path) {
        Err(why) => {
            println_stderr!("cicada: could not open {}: {}", display, why.description());
            return 0;
        }
        Ok(file) => file,
    };
    file.into_raw_fd()
}

#[cfg(test)]
mod tests {
    use super::extend_bandband;
    use super::is_alias;
    use shell;

    #[test]
    fn test_is_alias() {
        assert!(is_alias("alias ls='ls -lh'"));
    }

    #[test]
    fn test_extend_bandband() {
        let mut sh = shell::Shell::new();
        sh.previous_cmd = "foo".to_string();

        let mut line = "echo !!".to_string();
        extend_bandband(&sh, &mut line);
        assert_eq!(line, "echo foo");

        line = "echo \"!!\"".to_string();
        extend_bandband(&sh, &mut line);
        assert_eq!(line, "echo \"foo\"");

        line = "echo '!!'".to_string();
        extend_bandband(&sh, &mut line);
        assert_eq!(line, "echo '!!'");

        line = "echo '!!' && echo !!".to_string();
        extend_bandband(&sh, &mut line);
        assert_eq!(line, "echo '!!' && echo foo");
    }
}
