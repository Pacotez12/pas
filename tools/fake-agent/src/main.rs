use std::io::{self, BufRead, BufWriter, Write};
use std::thread;
use std::time::Duration;

fn main() {
    let stdin = io::stdin();
    let mut out = BufWriter::new(io::stdout().lock());

    writeln!(out, "fake-agent listo").ok();
    write!(out, "> ").ok();
    out.flush().ok();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            write!(out, "> ").ok();
            out.flush().ok();
            continue;
        }

        if let Some(text) = line.strip_prefix("echo ") {
            writeln!(out, "{text}").ok();
        } else if let Some(ms) = line.strip_prefix("sleep ") {
            if let Ok(ms) = ms.trim().parse::<u64>() {
                thread::sleep(Duration::from_millis(ms));
            }
            writeln!(out, "listo").ok();
        } else if let Some(n) = line.strip_prefix("flood ") {
            if let Ok(n) = n.trim().parse::<usize>() {
                for i in 1..=n {
                    writeln!(out, "línea {i}").ok();
                }
            }
        } else if let Some(code) = line.strip_prefix("exit ") {
            let code = code.trim().parse::<i32>().unwrap_or(0);
            out.flush().ok();
            std::process::exit(code);
        } else if let Some(name) = line.strip_prefix("env ") {
            let val = std::env::var(name.trim()).unwrap_or_else(|_| "(vacío)".into());
            writeln!(out, "{val}").ok();
        } else if line == "size" {
            let output = std::process::Command::new("stty")
                .arg("size")
                .stdin(std::process::Stdio::inherit())
                .output();
            match output {
                Ok(o) if o.status.success() => {
                    let text = String::from_utf8_lossy(&o.stdout);
                    writeln!(out, "{}", text.trim()).ok();
                }
                _ => {
                    writeln!(out, "0 0").ok();
                }
            }
        }

        write!(out, "> ").ok();
        out.flush().ok();
    }
}
