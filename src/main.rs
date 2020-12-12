#![feature(async_closure)]

use calamine::{open_workbook, Reader, Xlsx, DataType};
use regex::Regex;
use scraper::{Html, Selector};
use dotenv::dotenv;
use futures::StreamExt;

// http://www.helios825.org/url-parameters.php

const PLACEHOLDER: &str = "Platzhalter (fehlende Setnummer oder UVP)";

fn find_column( header_row: &[DataType], column_name: &str) -> usize {
    header_row.iter().enumerate().find( |( _index, item )| {
        return !item.is_empty() && item.get_string() == Some( column_name );
    } ).expect( &format!( "couldn't find column '{}'", &column_name ) ).0
}

fn read_portfolio( path: &dyn AsRef<std::path::Path> ) -> Result<Vec<Option<Item>>, Box<dyn std::error::Error>> {
    let mut excel: Xlsx<_> = open_workbook( path )?;
    if let Some(Ok(r)) = excel.worksheet_range("Tabelle1") {
        let first_row = &r.rows().next().expect( "portfolio needs at least 1 row including the column names" );
        let set_number_index = find_column( first_row, "Setnummer" );
        let target_price_index = find_column( first_row, "UVP LEGO" );

        Ok(r.rows()
            .skip(1)
            .map(|row| {
                if row[set_number_index].is_empty() || row[target_price_index].is_empty() {
                    return None;
                }

                let id = row[set_number_index].get_float().unwrap().to_string();
                println!("item: {}, price: {}", id, row[target_price_index].get_float().unwrap());
                Some( Item {
                    set_number: id,
                    target_price: row[target_price_index].get_float().unwrap(),
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

#[derive(Debug, Clone)]
struct Item {
    target_price: f64,
    set_number: String,
}

async fn get_ebay_request_id() -> Result<String, Box<dyn std::error::Error>> {
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

fn analyze_crawled_results(item: &Item, mut results: Vec<ItemResult>) -> Option<ItemAnalysis> {
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
        "item: {}   current: {:.2}€",
        &item.set_number,
        &result.avg,
    );
    Some(result)
}

fn collect_plausible_entries( document: &Html ) -> Vec<ItemResult> {
    let selector = Selector::parse("li.s-item").expect("Can't parse selector");
    document
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
        .collect()
}

async fn determine_current_value(item: Item, id: &str) -> Result<(String, f64), Box<dyn std::error::Error>> {
    let url = format!( "http://www.ebay.de/sch/i.html?_from=R40&_trksid={}&_nkw=Lego+{}&_ipg=200&LH_Sold=1&_sop=1&LH_ItemCondition=3",
            id, item.set_number ).to_string();
    let document = search_link(&url).await?;
    let results = collect_plausible_entries( &document );
    analyze_crawled_results(&item, results)
        .and_then( |res| Some( (item.set_number.clone(), res.avg) ) )
        .ok_or( String::from( "Couldn't analyze item results." ).into() )
}

async fn determine_current_value_robust(item: Item, id: &str) -> Result<(String, f64), Box<dyn std::error::Error>> {
    let mut i = 0;
    let mut res = Ok( (PLACEHOLDER.to_string(), 0.0 ) );
    while true {
        println!("request item {} {}", item.set_number, i);
        res = determine_current_value( item.clone(), id ).await;
        if res.is_ok() || i == 5 {
            return res;
        }
        i += 1;
    }

    res
}

fn create_csv( data: &Vec<(String, f64)> ) -> String {
    let header = "price in €\n".to_string();
    let content = data.iter().map( |(set, val)| {
        if set == PLACEHOLDER {
            PLACEHOLDER.to_string() + ","
        } else {
            format!( "{:.2}", val )
        }
    } ).collect::<Vec<_>>().join( "\n" );
    header + &content
}

async fn download_portfolio( url: &str ) -> Result<Vec<Option<Item>>, Box<dyn std::error::Error>> {
    use std::io::Write;
    let response = reqwest::Client::builder()
        .build()
        .expect("Can't create header")
        .get(url)
        .send()
        .await?
        .bytes()
        .await?;

    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all( &response as &[u8] )?;
    read_portfolio( &file.path() )
}

fn send_email_with_result( data: &[u8] ) {
    use lettre::smtp::authentication::Credentials;
    use lettre::{SmtpClient, Transport};
    use lettre_email::Email;

    let email = Email::builder()
    .from( dotenv::var( "FROM_EMAIL" ).unwrap() )
    .to( dotenv::var( "TO_EMAIL" ).unwrap() )
    .subject("Lego-Portfolio-Analyse")
    .attachment(data, "analysis_result.csv", &mime::STAR_STAR).unwrap()
    .html("<h1>Lego Portfolio Auswertung erfolgreich</h1>")
    .text("Ergenisse sind im Anhang")
    .build().unwrap();
    
    let creds = Credentials::new(dotenv::var( "FROM_EMAIL" ).unwrap(), dotenv::var( "FROM_EMAIL_PASSWORD" ).unwrap());
    
    // Open a remote connection to gmail
    let mut mailer = SmtpClient::new_simple("smtp.gmail.com")
    .unwrap()
    .credentials(creds)
    .transport();
    
    // Send the email
    match mailer.send(email.into()) {
        Ok(_) => println!("Email sent successfully!"),
        Err(e) => panic!("Could not send email: {:?}", e),
    }
    
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let start = chrono::Local::now();
    dotenv().ok();

    let id = &get_ebay_request_id().await?;
    let analzye_items = download_portfolio( dotenv::var( "PORTFOLIO_LINK" ).expect( ".env contains PORTFOLIO_LINK" ).as_str() ).await?
        .into_iter().map( async move |item| {
                match item {
                    Some( item ) => determine_current_value_robust(item, &id).await.unwrap(),
                    None => (PLACEHOLDER.to_string(), 0.0)
                }
            });

    let analysis = futures::stream::iter( analzye_items ).buffer_unordered( num_cpus::get() ).collect().await;
    let result = create_csv( &analysis );
    send_email_with_result( result.as_bytes() );
    println!(
        "computed current portfolio value in {} seconds.",
        chrono::Local::now()
            .signed_duration_since(start)
            .num_seconds()
    );
    Ok(())
}

