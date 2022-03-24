use clap::Parser;
use color_eyre::eyre::Context;
use rayon::prelude::*;
use std::{
    fs::File,
    io::{ErrorKind as ioErrorKind, Read},
    process::Command,
    str,
};

const ELF_HEADER: [u8; 4] = [0x7f, b'E', b'L', b'F'];

#[derive(Debug, Parser)]
struct Arg {
    /// print path to elf files
    #[clap(short, long)]
    file: bool,

    /// print average build time of package using qlop
    #[clap(short, long)]
    time: bool,

    /// print rebuild package command line
    #[clap(short, long)]
    rebuild: bool,

    /// only process one package (CAT/PN or PN without PV)
    atom: Option<String>,
}

fn eix(input: Option<String>) -> Vec<String> {
    let mut cmd = Command::new("eix");
    cmd.arg("-I#");
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
            .map(|s| s.to_owned())
            .collect()
    }
}

fn is_elf_file(path: &str) -> bool {
    let mut file = File::open(&path)
        .with_context(|| format!("Failed to open file: {path}"))
        .unwrap();
    let mut buf = [0u8; 4];
    match file.read_exact(&mut buf) {
        Ok(_) => buf == ELF_HEADER,
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

fn qlop(pkg: &str) -> u64 {
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

fn list_binary(pkg: &str) -> Vec<String> {
    qlist(pkg)
        .into_par_iter()
        .filter(|p| is_elf_file(&p))
        .collect()
}

fn have_binary(pkg: &str) -> bool {
    qlist(pkg).into_par_iter().any(|p| is_elf_file(&p))
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

fn main() {
    let opt = Arg::parse();
    color_eyre::install().unwrap();
    let need_time = opt.time || opt.rebuild;
    let pkgs = eix(opt.atom);
    let mut pkgs: Vec<_> = pkgs
        .into_par_iter()
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
                let time = if need_time {
                    qlop(&pkg)
                } else {
                    0
                };
                Some((pkg, list, time))
            } else {
                None
            }
        })
        .collect();
    if !opt.time && !opt.rebuild {
        return;
    }
    if need_time {
        pkgs.sort_by_key(|p| p.2);
    }
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
    if !opt.rebuild {
        return;
    }
    let small_pkgs:Vec<_> = pkgs.par_iter().filter_map(|p| {
        if p.2 < 60 {
            Some(p.0.to_owned())
        }else {
            None
        }
    }).collect();
    let middle_pkgs: Vec<_> = pkgs.par_iter().filter_map(|p| {
        if p.2 >= 60 && p.2 < 15*60 {
            Some(p.0.to_owned())
        }else {
            None
        }
    }).collect();
    let big_pkgs: Vec<_> = pkgs.par_iter().filter_map(|p| {
        if p.2 >= 15*60 {
            Some(p.0.to_owned())
        }else {
            None
        }
    }).collect();
    println!("emerge -av1j16 -l20 --keep-going {}", small_pkgs.join(" "));
    println!("emerge -av1j2 -l20 --keep-going {}", middle_pkgs.join(" "));
    println!("emerge -av1 --keep-going {}", big_pkgs.join(" "));
}
