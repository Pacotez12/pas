use std::io::{self, BufRead, BufWriter, Read, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

fn handle_line(line: &str, out: &mut BufWriter<io::StdoutLock<'_>>) {
    let line = line.trim();
    if line.is_empty() {
        write!(out, "> ").ok();
        out.flush().ok();
        return;
    }

    if let Some(text) = line.strip_prefix("echo ") {
        writeln!(out, "{text}").ok();
    } else if let Some(question) = line.strip_prefix("ask ") {
        write!(out, "{question}").ok();
        out.flush().ok();
        return;
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
        let output = Command::new("stty")
            .arg("size")
            .stdin(Stdio::inherit())
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

fn main() {
    let drop_inputs: usize = std::env::var("FAKE_AGENT_DROP_INPUTS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);

    if drop_inputs > 0 {
        let _ = Command::new("stty")
            .args(["-echo", "-icanon", "min", "1", "time", "0"])
            .stdin(Stdio::inherit())
            .status();
    }

    let stdin = io::stdin();
    let mut out = BufWriter::new(io::stdout().lock());

    writeln!(out, "fake-agent listo").ok();
    write!(out, "> ").ok();
    out.flush().ok();

    if drop_inputs == 0 {
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            handle_line(&line, &mut out);
        }
        return;
    }

    let mut drop_remaining = drop_inputs;
    let mut line_buf = Vec::new();
    let mut buf = [0u8; 4096];
    let mut reader = stdin.lock();
    while let Ok(n) = reader.read(&mut buf) {
        if n == 0 {
            break;
        }
        if drop_remaining > 0 {
            drop_remaining -= 1;
            continue;
        }
        for &b in &buf[..n] {
            if b == b'\r' || b == b'\n' {
                writeln!(out).ok();
                out.flush().ok();
                let line = String::from_utf8_lossy(&line_buf).into_owned();
                line_buf.clear();
                handle_line(&line, &mut out);
            } else {
                line_buf.push(b);
                out.write_all(&[b]).ok();
                out.flush().ok();
            }
        }
    }
}
