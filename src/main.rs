use clap::Parser;
use color_eyre::eyre::Context;
use rayon::prelude::*;
use std::{
    collections::HashSet,
    fs::File,
    io::{ErrorKind as ioErrorKind, Read},
    process::Command,
    str,
};

const ELF_HEADER: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const ARCHIVE_HEADER: [u8; 8] = *b"!<arch>\n";

#[derive(Debug, Parser)]
struct Arg {
    /// print path to elf files
    #[clap(short, long)]
    file: bool,

    /// print average build time of package using qlop
    #[clap(short, long)]
    time: bool,

    /// print rebuild package command line
    #[clap(short = 'b', long)]
    rebuild: bool,

    /// Exclude packages build after this package
    /// useful for recovery from a build failure
    #[clap(short, long)]
    recovery: Option<String>,

    /// only process one package (CAT/PN or PN without PV)
    #[clap(short, long)]
    atom: Option<String>,
}

fn eix(input: Option<String>) -> Vec<String> {
    let mut cmd = Command::new("eix");
    cmd.arg("-I");
    cmd.arg("--format");
    cmd.arg("<installedversions:EQNAMEVERSION>");
    if let Some(input) = input {
        cmd.arg(input);
    }
    let output = cmd.output().unwrap();
    if !output.status.success() {
        if output.status.code() == Some(1) {
            Vec::new()
        } else {
            panic!(
                "eix failed!, stdout:\n{:?}\nstderr:\n{:?}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        }
    } else {
        let output = str::from_utf8(&output.stdout).unwrap();
        output
            .split_terminator('\n')
            .filter(|s| !s.starts_with("[")) // [1] "xxx" /xxx/xxx/
            .filter(|s| !s.is_empty())
            .filter(|s| !s.starts_with("Found ")) // Found xxx matches
            .map(|s| s.to_owned())
            .collect()
    }
}

fn is_elf_or_archive_file(path: &str) -> bool {
    let Ok(mut file) = File::open(&path) else {
        eprintln!("Failed to open file: {path}");
        return false;
    };
    let mut buf = [0u8; 8];
    match file.read_exact(&mut buf) {
        Ok(_) => {
            let is_elf = buf[..4] == ELF_HEADER;
            let is_archive = buf == ARCHIVE_HEADER;
            is_elf || is_archive
        }
        Err(e) => {
            if e.kind() == ioErrorKind::UnexpectedEof {
                false
            } else {
                Err(e)
                    .with_context(|| format!("Failed to read file: {path}"))
                    .unwrap()
            }
        }
    }
}

fn qlist(pkg: &str) -> Vec<String> {
    let mut cmd = Command::new("qlist");
    cmd.arg("-eo");
    cmd.arg(pkg);
    let output = cmd.output().unwrap();
    if !output.status.success() {
        panic!(
            "qlist {pkg} failed!, stdout:\n{:?}\nstderr:\n{:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    }
    let output = str::from_utf8(&output.stdout).unwrap();
    output
        .split_terminator('\n')
        .map(|s| s.to_owned())
        .collect()
}

fn qlop_time(pkg: &str) -> u64 {
    let mut cmd = Command::new("qlop");
    cmd.arg("-CMamq");
    cmd.arg(pkg);
    let output = cmd.output().unwrap();
    if !output.status.success() {
        panic!(
            "qlop {pkg} failed!, stdout:\n{:?}\nstderr:\n{:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    }
    let output = str::from_utf8(&output.stdout).unwrap();
    let num = output.split_whitespace().nth(1).unwrap_or("0");
    num.parse().unwrap()
}

fn qlop_after(pkg: &str) -> Vec<String> {
    let mut cmd = Command::new("qlop");
    cmd.arg("-CMqv");
    cmd.arg("-d7day");
    cmd.arg("--merge");
    let output = cmd.output().unwrap();
    if !output.status.success() {
        panic!(
            "qlop {pkg} failed!, stdout:\n{:?}\nstderr:\n{:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    }
    let output = str::from_utf8(&output.stdout).unwrap();
    let set: Vec<&str> = output.split_terminator('\n').collect();
    let set = set
        .rsplit(|p| p == &pkg)
        .next()
        .expect("You don't build gaving package in last 3 days");
    set.into_iter().map(|s| format!("={s}")).collect()
}

fn list_binary(pkg: &str) -> Vec<String> {
    qlist(pkg)
        .into_par_iter()
        .filter(|p| is_elf_or_archive_file(&p))
        .collect()
}

fn have_binary(pkg: &str) -> bool {
    qlist(pkg)
        .into_par_iter()
        .any(|p| is_elf_or_archive_file(&p))
}

fn print_time(sec: u64) -> String {
    let min = sec / 60;
    if min != 0 {
        let sec = sec % 60;
        format!("{min}′{sec}″")
    } else {
        format!("{sec}s")
    }
}

fn get_broken(portage: &str) -> HashSet<String> {
    let mut cmd = Command::new("fd");
    cmd.arg("-d2");
    cmd.arg("-td");
    cmd.current_dir(portage);
    let output = cmd.output().unwrap();
    if !output.status.success() {
        panic!(
            "fd -d2 -td {portage} failed!, stdout:\n{:?}\nstderr:\n{:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    }
    let output = str::from_utf8(&output.stdout).unwrap();
    output
        .split_terminator('\n')
        .map(|s| &s[..s.len() - 1])
        .filter(|s| s.contains('/'))
        .map(|s| format!("={s}"))
        .collect()
}

fn main() {
    let opt = Arg::parse();
    color_eyre::install().unwrap();
    let need_time = opt.time || opt.rebuild;
    let pkgs = eix(opt.atom);
    let mut skip = get_broken("/tmp/portage");
    for p in skip.iter() {
        println!("Skip broken {p}")
    }
    if let Some(p) = opt.recovery.as_deref() {
        let done = qlop_after(p);
        for p in done.iter() {
            println!("Skip built {p}")
        }
        skip.extend(done)
    }
    let mut pkgs: Vec<_> = pkgs
        .into_par_iter()
        .filter(|p| !skip.contains(p))
        .filter_map(|pkg| {
            let (mut list, have) = if opt.file {
                let list = list_binary(&pkg);
                let have = !list.is_empty();
                (list, have)
            } else {
                (Vec::new(), have_binary(&pkg))
            };
            if have {
                list.par_sort();
                let time = if need_time { qlop_time(&pkg) } else { 0 };
                Some((pkg, list, time))
            } else {
                None
            }
        })
        .collect();
    if need_time {
        pkgs.sort_by_key(|p| p.2);
    }
    if opt.recovery.is_none() {
        for (pkg, list, time) in &pkgs {
            if need_time {
                let t = print_time(*time);
                println!("{pkg}: {t}");
            } else {
                println!("{pkg}");
            }
            if opt.file {
                for f in list {
                    println!("{f}")
                }
            }
        }
    }
    if !opt.rebuild {
        return;
    }
    let small_pkgs: Vec<_> = pkgs
        .par_iter()
        .filter_map(|p| if p.2 < 60 { Some(p.0.to_owned()) } else { None })
        .map(|p| format!("'{p}'"))
        .collect();
    let middle_pkgs: Vec<_> = pkgs
        .par_iter()
        .filter_map(|p| {
            if p.2 >= 60 && p.2 < 15 * 60 {
                Some(p.0.to_owned())
            } else {
                None
            }
        })
        .map(|p| format!("'{p}'"))
        .collect();
    let big_pkgs: Vec<_> = pkgs
        .par_iter()
        .filter_map(|p| {
            if p.2 >= 15 * 60 {
                Some(p.0.to_owned())
            } else {
                None
            }
        })
        .map(|p| format!("'{p}'"))
        .collect();
    println!(
        "sudo emerge -av1 -j16 -l20 --keep-going {}",
        small_pkgs.join(" ")
    );
    println!(
        "sudo emerge -av1 -j2 -l20 --keep-going {}",
        middle_pkgs.join(" ")
    );
    println!("sudo emerge -av1 --keep-going {}", big_pkgs.join(" "));
}
