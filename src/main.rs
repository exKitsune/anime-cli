#[cfg(all(feature = "with-mpv", feature = "with-opener"))]
compile_error!("Feature `with-mpv` or `with-opener` shouldn't be enabled both.");

use std::process::exit;

mod anime_dl;
#[cfg(feature = "find")]
mod anime_find;
#[cfg(feature = "play")]
mod play;

#[cfg(not(feature = "find"))]
fn main() {
    let args: Vec<String> = std::env::args().collect(); // We collect args here

    if args.len() > 1 && args[1].starts_with("/msg"){
        let len = args.len() - 1;
        let mut bots = Vec::with_capacity(len);
        let mut packages = Vec::with_capacity(len);
        for ep in 1..=len {
            let msg = args[ep].trim_start_matches("/msg ");
            let package: Vec<&str> = msg.splitn(2, " xdcc send #").collect();
            bots.push(package[0].to_string());
            packages.push(package[1].to_string());
        }
        let irc_request = anime_dl::IRCRequest {
            bots,
            packages,
            ..Default::default()
        };
        if let Err(e) = anime_dl::connect_and_download(irc_request, None) {
            eprintln!("{}", e);
            exit(1);
        };
    }
}

#[cfg(feature = "find")]
fn main() {
    let args: Vec<String> = std::env::args().collect(); // We collect args here

    let cli = args.len() > 1;

    let (bots, packages, dir_path) = if cli && args[1].starts_with("/msg"){
        let len = args.len() - 1;
        let mut bots = Vec::with_capacity(len);
        let mut packages = Vec::with_capacity(len);
        for ep in 1..=len {
            let msg = args[ep].trim_start_matches("/msg ");
            let package: Vec<&str> = msg.splitn(2, " xdcc send #").collect();
            bots.push(package[0].to_string());
            packages.push(package[1].to_string());
        }
        (bots, packages, None)
    } else {
        anime_find::find(args, cli)
    };
    let irc_request = anime_dl::IRCRequest {
        bots,
        packages,
        ..Default::default()
    };
    if let Err(e) = anime_dl::connect_and_download(irc_request, dir_path) {
        eprintln!("{}", e);
        exit(1);
    };
}
