use clap::{crate_version, App, Arg};
use http2::client::Client;
use url::Url;

fn main() {
    env_logger::init();

    let matches = App::new("http2")
        .version(crate_version!())
        .arg(Arg::with_name("url").required(true).index(1))
        .get_matches();
    let url = Url::parse(matches.value_of("url").expect("missing url")).expect("invalid url");

    let client = Client::default();
    match client.get(url) {
        Ok(response) => println!("{}", String::from_utf8_lossy(&response.body)),
        Err(err) => eprintln!("{:#?}", err),
    }
}
