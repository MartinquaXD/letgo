#![feature(async_closure)]

use calamine::{open_workbook, Reader, Xlsx};
use futures::future::join_all;
use regex::Regex;
use tokio::prelude::*;
use scraper::{Html, Selector};

// http://www.helios825.org/url-parameters.php

fn read_portfolio2() -> Result<Vec<Item>, Box<dyn std::error::Error>> {
    let mut excel: Xlsx<_> = open_workbook("Mappe.xlsx")?;
    if let Some(Ok(r)) = excel.worksheet_range("Tabelle1") {
        let mut unique_items = Vec::new();

        Ok(r.rows()
            .skip(1)
            .filter_map(|row| {
                if row[1].is_empty() || row[3].is_empty() {
                    return None;
                }

                let id = row[1].get_float().unwrap().to_string();
                if unique_items.iter().any(|item| item == &id) {
                    return None;
                }

                unique_items.push(id.clone());
                Some( Item {
                    set_number: id,
                    target_price: row[3].get_float().unwrap(),
                    bought: row[4].get_float().unwrap()
                } )
            })
            .collect())
    } else {
        Ok(vec![])
    }
}

async fn search_link(link: &str) -> Result<Html, Box<dyn std::error::Error>> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::USER_AGENT,
        reqwest::header::HeaderValue::from_static("curl/7.71.1"),
    );
    headers.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("*/*"),
    );

    let response = reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .expect("Can't create header")
        .get(link)
        .send()
        .await?
        .text()
        .await?;

    Ok(Html::parse_document(&response))
}

#[derive(Debug)]
struct Item {
    bought: f64,
    target_price: f64,
    set_number: String,
}

async fn get_request_id() -> Result<String, Box<dyn std::error::Error>> {
    let start = search_link("http://www.ebay.de").await?;
    let sel =
        Selector::parse("input[type='hidden'][name='_trksid']").expect("Can't parse selector");
    Ok(start
        .select(&sel)
        .next()
        .unwrap()
        .value()
        .attr("value")
        .unwrap()
        .to_string())
}

#[derive(Debug, Clone)]
struct ItemResult {
    price: f64,
    date: chrono::DateTime<chrono::FixedOffset>,
    name: String,
}

pub fn parse_date(mut date: String) -> chrono::DateTime<chrono::FixedOffset> {
    date.retain(|c| c != '.');
    let mut parts = date.split(' ');
    let day = parts.next().unwrap().to_string();
    let month = match parts.next().unwrap() {
        "Jan" => "01",
        "Feb" => "02",
        "Mar" => "03",
        "Apr" => "04",
        "Mai" => "05",
        "Jun" => "06",
        "Jul" => "07",
        "Aug" => "08",
        "Sep" => "09",
        "Okt" => "10",
        "Nov" => "11",
        "Dez" => "12",
        _ => "-1",
    };
    let time = parts.next().unwrap();

    let corrected_date = format!("{}-{}-{} {} +0000", day, month, 2020, time);
    chrono::DateTime::parse_from_str(&corrected_date, "%d-%m-%Y %H:%M %z").unwrap()
}

struct ItemAnalysis {
    min: f64,
    max: f64,
    avg: f64,
    data_points: usize,
}

fn analyze_results(item: &Item, mut results: Vec<ItemResult>) -> Option<ItemAnalysis> {
    let mut result = ItemAnalysis {
        min: f64::MAX,
        max: 0.0,
        avg: 0.0,
        data_points: 0,
    };
    let set_regex = Regex::new(r"\d{5,}").unwrap();
    results.retain(|result| {
        let is_recent =
            chrono::Local::now().signed_duration_since(result.date) < chrono::Duration::days(30);
        let price_okay = result.price > item.target_price * 0.5;
        let name_included = result.name.contains(&item.set_number);
        let set_numbers = set_regex.find_iter(&result.name).count();
        let at_most_set_present = if set_numbers == 0 {
            true
        } else {
            name_included && set_numbers == 1
        };

        is_recent && price_okay && at_most_set_present
    });

    if results.is_empty() {
        return None;
    }

    let mut sum = 0.0;
    for sell in &results {
        result.min = result.min.min(sell.price);
        result.max = result.max.max(sell.price);
        sum += sell.price;
    }

    result.data_points = results.len();
    result.avg = sum / result.data_points as f64;
    println!(
        "item: {}   bought for: {}€  current: {:.2}€   gain: {:.0}% / {:.2} €",
        &item.set_number,
        &item.bought,
        &result.avg,
        ((&result.avg / &item.bought) * 100.0) - 100.0,
        &result.avg - &item.bought
    );
    Some(result)
}

async fn handle_portfolio_item(item: Item, id: &str) -> Result<(String, f64), Box<dyn std::error::Error>> {
    // if item.set_number != "42096" {
    //     return;
    // }

    let url = format!( "http://www.ebay.de/sch/i.html?_from=R40&_trksid={}&_nkw=Lego+{}&_ipg=200&LH_Sold=1&_sop=1&LH_ItemCondition=3",
            id, item.set_number ).to_string();

    let document = search_link(&url).await?;
    let selector = Selector::parse("li.s-item").expect("Can't parse selector");
    let results: Vec<_> = document
        .select(&selector)
        .filter_map(|ebay_item| {
            let price = ebay_item
                .select(
                    &Selector::parse("span.s-item__price>span.POSITIVE")
                        .expect("Can't parse selector"),
                )
                .next()
                .and_then(|price| {
                    let price_str = price.text().nth(0).unwrap();
                    let price_str = price_str.replace(",", ".");
                    price_str[4..].parse::<f64>().ok()
                });

            let date = ebay_item
                .select(
                    &Selector::parse("span.s-item__detail>span.s-item__ended-date")
                        .expect("Can't parse selector"),
                )
                .next()
                .and_then(|date| {
                    let date_str = date.text().nth(0).unwrap().to_string();
                    Some(parse_date(date_str))
                });

            let name = ebay_item
                .select(&Selector::parse("h3.s-item__title").expect("Can't parse selecor"))
                .next()
                .and_then(|item| Some(item.inner_html()))
                .or(Some("".to_string()));

            if let (Some(price), Some(date)) = (price, date) {
                Some(ItemResult {
                    price,
                    date,
                    name: name.unwrap(),
                })
            } else {
                None
            }
        })
        .collect();

    analyze_results(&item, results)
        .and_then( |res| Some( (item.set_number.clone(), res.avg) ) )
        .ok_or( String::from( "Couldn't analyze item results." ).into() )
}

async fn store_results( data: &Vec<(String, f64)> ) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = tokio::fs::File::create("analysis.csv").await?;
    let header = "set_number, price in €\n".to_string();
    let content = data.iter().map( |item| format!( "{}, {:.2}", item.0, item.1 ) ).collect::<Vec<_>>().join( "\n" );
    file.write_all( (header + &content).as_bytes() ).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let start = chrono::Local::now();

    let id = get_request_id().await?;
    let portfolio = read_portfolio2()?;

    let id2 = &id;
    let handle_portfolio: Vec<_> = portfolio
        .into_iter()
        .map(async move |item| handle_portfolio_item(item, &id2).await)
        .collect();
    let analysis: Vec<_> = join_all(handle_portfolio).await.into_iter().filter_map( |res| res.ok() ).collect();
    store_results( &analysis ).await?;
    println!(
        "computed current portfolio value in {} seconds.",
        chrono::Local::now()
            .signed_duration_since(start)
            .num_seconds()
    );
    Ok(())
}
