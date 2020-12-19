#![feature(async_closure)]

mod helper_types;
use helper_types::*;

use calamine::{open_workbook, Reader, Xlsx, DataType};
use regex::Regex;
use scraper::{Html, Selector};
use dotenv::dotenv;
use futures::StreamExt;
use std::collections::{ HashSet, HashMap };

type Error = Box<dyn std::error::Error>;
type MyResult<T> = Result<T, Error>;


fn find_column( header_row: &[DataType], column_name: &str) -> usize {
    header_row.iter().enumerate().find( |( _index, item )| {
        return !item.is_empty() && item.get_string() == Some( column_name );
    } ).expect( &format!( "couldn't find column '{}'", &column_name ) ).0
}

fn get_item_of_row( row: &[DataType], set_number: usize, target_price: usize ) -> MyResult<PortfolioItem> {
    if row[set_number].is_empty() && row[target_price].is_empty() {
        //returning an empty error will lead to an empty row in the csv down the line
        return Err( "".into() );
    }
    if row[set_number].is_empty() {
        return Err( "Set Nummer fehlt".into() );
    }
    if row[target_price].is_empty() {
        return Err( "UVP fehlt".into() );
    }

    Ok( PortfolioItem {
        set_number: row[set_number].get_float().unwrap().to_string(),
        target_price: row[target_price].get_float().unwrap(),
    } )
}

fn read_portfolio( path: &dyn AsRef<std::path::Path> ) -> MyResult<Vec<MyResult<PortfolioItem>>> {
    let mut excel: Xlsx<_> = open_workbook( path )?;
    if let Some(Ok(r)) = excel.worksheet_range("Tabelle1") {
        let first_row = &r.rows().next().expect( "portfolio needs at least 1 row including the column names" );
        let set_number_index = find_column( first_row, "Setnummer" );
        let target_price_index = find_column( first_row, "UVP LEGO" );
        let results = r.rows()
            .skip(1)
            .map(|row| get_item_of_row( row, set_number_index, target_price_index ) )
            .collect();

        Ok( results )
    } else {
        Ok(vec![])
    }
}

async fn search_link(link: &str) -> MyResult<Html> {
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

async fn get_ebay_request_id() -> MyResult<String> {
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
    let time = parts.next().expect( "Ebay result has time" );

    let corrected_date = format!("{}-{}-{} {} +0000", day, month, 2020, time);
    chrono::DateTime::parse_from_str(&corrected_date, "%d-%m-%Y %H:%M %z").unwrap()
}

fn analyze_crawled_results(item: &PortfolioItem, mut results: Vec<EbayResult>) -> MyResult<PriceAnalysis> {
    let mut result = PriceAnalysis::default();
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
        return Err( "Keine sinnvollen Ergebnisse gefunden".into() );
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
    Ok( result )
}

fn collect_plausible_entries( document: &Html ) -> Vec<EbayResult> {
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
                Some(EbayResult {
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

async fn determine_current_value(item: PortfolioItem, id: &str) -> MyResult<f64> {
    let url = format!( "http://www.ebay.de/sch/i.html?_from=R40&_trksid={}&_nkw=Lego+{}&_ipg=200&LH_Sold=1&_sop=1&LH_PortfolioItemCondition=3",
            id, item.set_number ).to_string();
    let document = search_link(&url).await?;
    let results = collect_plausible_entries( &document );
    analyze_crawled_results(&item, results).and_then( |res| Ok( res.avg ) )
}

async fn determine_current_value_robust(item: PortfolioItem, id: &str) -> MyResult<f64> {
    let mut i = 0u8;
    loop {
        let res = determine_current_value( item.clone(), id ).await;
        if res.is_ok() || i == 5 {
            return res;
        }
        i += 1;
    }
}

fn create_csv( portfolio: Vec<MyResult<PortfolioItem>>, data: &HashMap<String, MyResult<f64>> ) -> String {
    let header = "price in €\n".to_string();
    let content = portfolio.into_iter().map( |item| {
        match item {
            Err( error ) => error.to_string(),
            Ok( item ) => {
                match data.get( &item.set_number ) {
                    None => unreachable!(),
                    Some( Err( error ) ) => error.to_string(),
                    Some( Ok( price ) ) => format!( "{:.2}", price ).to_string()
                }
            }
        }
    } ).collect::<Vec<_>>().join( "\n" );
    header + &content
}

async fn download_portfolio( url: &str ) -> MyResult<Vec<MyResult<PortfolioItem>>> {
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
async fn main() -> MyResult<()> {
    let start = chrono::Local::now();
    dotenv().ok();

    let id = &get_ebay_request_id().await?;
    let links = dotenv::var( "PORTFOLIO_LINK" ).expect( ".env contains PORTFOLIO_LINK" ).to_string();
    for link in links.split( ' ' ) {
        let portfolio = download_portfolio( link ).await?;
        let mut unique_items: HashSet<PortfolioItem> = portfolio.iter().filter_map( |i| match i {
            Err( _ ) => None,
            Ok( val ) => Some( val.clone() )
        } ).collect();

        let fetch_items = unique_items.drain()
            .map( async move |item| ( item.set_number.clone(), determine_current_value_robust( item, &id ).await ) );

        let analysis: HashMap<_, _> = futures::stream::iter( fetch_items )
            .buffer_unordered( num_cpus::get() ).collect::<Vec<_>>().await
            .into_iter().collect::<HashMap<_,_>>();

        let result = create_csv( portfolio, &analysis );
        println!( "{}", result );
        send_email_with_result( result.as_bytes() );
        println!(
            "computed current portfolio value in {} seconds.",
            chrono::Local::now()
                .signed_duration_since(start)
                .num_seconds()
        );
    }
    Ok(())
}

