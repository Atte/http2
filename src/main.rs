use clap::{crate_version, App, Arg};
use http2::{Client, Request};
use url::Url;

#[tokio::main]
async fn main() {
    env_logger::init();

    let matches = App::new("http2")
        .version(crate_version!())
        .arg(
            Arg::with_name("url")
                .required(true)
                .multiple(true)
                .number_of_values(1)
                .validator(|url| {
                    Url::parse(&url).map_err(|err| err.to_string())?;
                    Ok(())
                }),
        )
        .get_matches();

    // unwrap: the parameter has already been validated by clap
    let urls = matches
        .values_of("url")
        .unwrap()
        .map(|url| Url::parse(url).unwrap());

    let client = Client::default();

    for url in urls {
        match client.request(Request::get(url)).await {
            Ok(response) => println!("{}", String::from_utf8_lossy(&response.body)),
            Err(err) => eprintln!("{:#?}", err),
        }
    }
}
