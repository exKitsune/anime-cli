use serde::Deserialize;
use anyhow::{Result, anyhow};

const API_URL: &str = "https://api.nibl.co.uk/nibl";

#[derive(Clone)]
pub struct DCCPackage {
    pub number: i32,
    pub bot: String,
    pub filename: String,
    pub sizekbits: i64,
}

pub fn find_package(query: &String, episode: &Option<u16>) -> Result<DCCPackage> {
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
        sizekbits: first_package.sizekbits,
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
    sizekbits: i64,
}
