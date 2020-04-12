use serde::Deserialize;
use anyhow::{Result, anyhow};
use std::fs;
use std::path::Path;
use std::ffi::OsStr;
use std::process::exit;
use std::io;
use std::path::PathBuf;
use getopts::Options;
#[cfg(feature = "play")]
use crate::play::play_video;

const API_URL: &str = "https://api.nibl.co.uk/nibl";

const AUDIO_EXTENSIONS: &'static [&'static str] = &["aif", "cda", "mid", "midi", "mp3",
                                                    "mpa", "ogg", "wav", "wma", "wpl"];

const VIDEO_EXTENSIONS: &'static [&'static str] = &["3g2", "3gp", "avi", "flv", "h264",
                                                    "m4v", "mkv", "mov", "mp4", "mpg",
                                                    "mpeg", "rm", "swf", "vob", "wmv"];
       
#[derive(Clone)]
pub struct DCCPackage {
    pub number: i32,
    pub bot: String,
    pub filename: String,
}

fn find_package(query: &String, episode: &Option<u16>) -> Result<DCCPackage> {
    let packages = match search_packages(query, episode) {
        Ok(p) => p,
        Err(e) => return Err(anyhow!("Error while fetching results: {}", e)),
    };

    let first_package = match packages.first() {
        Some(p) => p,
        _ => return Err(anyhow!("Could not find any result for this query.")),
    };

    let bot_name = match find_bot_name(&first_package.bot_id) {
        Ok(b) => b,
        Err(e) => return Err(e),
    };

    Ok(DCCPackage {
        bot: bot_name.to_string(),
        number: first_package.number,
        filename: first_package.name.clone(),
    })
}

fn search_packages(query: &String, episode: &Option<u16>) -> Result<Vec<Package>> {
    let mut search_url = format!("{}/search?query={}", API_URL, query);
    if let Some(episode) = episode {
        search_url.push_str("&episodeNumber=");
        search_url.push_str(&episode.to_string());
    }
    let search_result: SearchResult = reqwest::blocking::get(&search_url)?.json()?;
    if search_result.status == "OK" {
        Ok(search_result.content)
    } else {
        Err(anyhow!("Could not search package: {}", search_result.message))
    }
}

fn find_bot_name(id: &i64) -> Result<String> {
    get_bot_list().and_then(|bot_list| {
        if let Some(bot) = bot_list.iter().find(|bot| &bot.id == id) {
            Ok(bot.name.to_string())
        } else {
            Err(anyhow!("Results found, but unknown bot."))
        }
    })
}

fn get_bot_list() -> Result<Vec<Bot>> {
    let bot_list: BotList = reqwest::blocking::get(&format!("{}/bots", API_URL))?.json()?;
    if bot_list.status == "OK" {
        Ok(bot_list.content)
    } else {
        Err(anyhow!("Could not fetch bot list: {}", bot_list.message))
    }
}

fn print_usage(program: &str, opts: Options) {
    let msg = opts.short_usage(&program);
    print!("{}", opts.usage(&msg));
    println!("\n\
    ===================================\n\
    Helpful Tips:                      \n\
    Try to keep your anime name simple \n\
    and use quotes when you use -q     \n\
    e.g. \"sakamoto\"                  \n\
                                       \n\
    Common resolutions 480/720/1080    \n\
                                       \n\
    e.g. -e 1 -t 10                    \n\
    everything from 1 -------> 10      \n\
                                       \n\
    You can apply default resolution   \n\
    and default to # with a blank      \n\
    ===================================\n
    ");
}

fn get_cli_input(prompt: &str) -> String {
    println!("{}", prompt);
    let mut input = String::new();
    if let Err(e) = io::stdin().read_line(&mut input) {
        eprintln!("{}", e);
        eprintln!("Please enter a normal query");
        exit(1);
    }
    input.to_string().replace(|c: char| c == '\n' || c == '\r', "")
}

fn parse_number(str_num: String) -> Option<u16> {
    let c_str_num = str_num.replace(|c: char| !c.is_numeric(), "");
    match c_str_num.parse::<u16>() {
        Ok(e) => Some(e),
        Err(err) => {
            if err.to_string() == "cannot parse integer from empty string" {
                None
            } else {
                eprintln!("Input must be numeric.");
                exit(1);
            }
        }
    }
}

pub fn find(args: Vec<String>, cli: bool) -> (Vec<String>, Vec<String>, Option<PathBuf>) {
    let mut query: String = String::new();
    let resolution: Option<u16>;
    let mut episode: Option<u16> = None;
    let mut last_ep: Option<u16> = None;

    let mut _is_play: bool = false;

    // Are we in cli mode or prompt mode?
    if cli {
        let program = args[0].clone();
        let mut opts = Options::new();
        opts.optopt("q", "query", "Query to run", "QUERY")
            .optopt("e", "episode", "Start from this episode", "NUMBER")
            .optopt("t", "to", "Last episode", "NUMBER")
            .optopt("r", "resolution", "Resolution", "NUMBER");
        if cfg!(feature = "play") {
            opts.optflag("p", "play", "Open with a player");
        }
        opts.optflag("h", "help", "print this help menu");
    
        let matches = match opts.parse(&args[1..]) {
            Ok(m) => m,
            Err(error) => {
                eprintln!("{}.", error);
                eprintln!("{}", opts.short_usage(&program));
                exit(1);
            }
        };
    
        // Unfortunately, cannot use getopts to check for a single optional flag
        // https://github.com/rust-lang-nursery/getopts/issues/46
        if matches.opt_present("h") {
            print_usage(&program, opts);
            exit(0)
        }

        if let Some(q) = matches.opt_str("q") {
            query = q;
            resolution = match matches.opt_str("r").as_ref().map(String::as_str) {
                Some("0") => None,
                Some(r) => parse_number(String::from(r)),
                _ => Some(720),
            };
            if let Some(ep) = matches.opt_str("e") {
                episode = parse_number(ep)
            }
            if let Some(t) = matches.opt_str("t") {
                last_ep = parse_number(t)
            }
            if cfg!(feature = "play") {
                _is_play = matches.opt_present("p");
            }
        } else {
            eprintln!("query is needed.");
            exit(0)
        }
    } else {
        println!("Welcome to anime-cli");
        if cfg!(feature = "play") {
            println!("Default: resolution => None | episode => None | to == episode | play => false");
        } else {
            println!("Default: resolution => None | episode => None | to == episode");
        }
        println!("Resolution shortcut: 1 => 480p | 2 => 720p | 3 => 1080p");
        while query.is_empty() {
            query = get_cli_input("Anime/Movie name: ");
        }
    
        resolution = match parse_number(get_cli_input("Resolution: ")) {
            Some(1) => Some(480),
            Some(2) => Some(720),
            Some(3) => Some(1080),
            x => x,
        };
        episode = parse_number(get_cli_input("Start from the episode: "));
        last_ep = match parse_number(get_cli_input("To this episode: ")) {
            None if episode.is_some() => episode,
            x => x,
        };
        if cfg!(feature = "play") {
            _is_play = get_cli_input("Play now? [y/N]: ").to_ascii_lowercase().eq("y");
        }
    }

    // Make sure last episode isn't smaller than episode start
    last_ep.and_then(|t| {
        if t < episode.unwrap_or(1) {
            std::mem::swap(&mut episode, &mut last_ep); // swap them
        }
        Some(())
    });

    // If resolution entered, add a resolution to the query
    if let Some(res) = resolution {
        query.push(' ');
        query.push_str(&res.to_string());
    }

    let mut dccpackages = vec![];

    let mut num_episodes = 0;  // Search for packs, verify it is media, and add to a list
    for i in episode.unwrap_or(1)..last_ep.unwrap_or(episode.unwrap_or(1)) + 1 {
        if episode.is_some() || last_ep.is_some() {
            println!("Searching for {} episode {}", query, i);
        } else {
            println!("Searching for {}", query);
        }
        match find_package(&query, &episode.or(last_ep).and(Some(i))) {
            Ok(p) => {
                match Path::new(&p.filename).extension().and_then(OsStr::to_str) {
                    Some(ext) => {
                        if !AUDIO_EXTENSIONS.contains(&ext) && !VIDEO_EXTENSIONS.contains(&ext) {
                            eprintln!("Warning, this is not a media file! Skipping");
                        } else {
                            dccpackages.push(p);
                            num_episodes += 1;
                        }
                    },
                    _ => { eprintln!("Warning, this file has no extension, skipping"); }
                }
            },
            Err(e) => {
                eprintln!("{}", e);
            }
        };
    }

    if num_episodes == 0 { exit(1); }

    match fs::create_dir(&query) { // organize
        Ok(_) => println!{"Created folder {}", &query},
        _ => eprintln!{"Could not create a new folder, does it exist?"},
    };
    let dir_path = Path::new(&query).to_owned();

    let bots = dccpackages.clone().into_iter().map(|package| package.bot).collect();
    let packages = dccpackages.clone().into_iter().map(|package| package.number.to_string()).collect();

    #[cfg(feature = "play")]
    {
        //If we don't have mpv, we'll open the file using default media app. We can't really hook into it so we limit to 1 file so no spam
        let video_handle = if _is_play && (num_episodes == 1 || cfg!(feature = "with-mpv")) {
            Some(play_video(dccpackages.into_iter().map(|package| package.filename).collect(), dir_path.clone()))
        } else {
            None
        };

        if let Some(vh) = video_handle {
            vh.join().unwrap();
        }
    }

    (bots, packages, Some(dir_path))
}

#[derive(Deserialize)]
struct BotList {
    status: String,
    message: String,
    content: Vec<Bot>,
}

#[derive(Deserialize)]
struct Bot {
    id: i64,
    name: String,
}

#[derive(Deserialize)]
struct SearchResult {
    status: String,
    message: String,
    content: Vec<Package>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Package {
    bot_id: i64,
    number: i32,
    name: String,
}
