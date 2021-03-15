use clap::{App, Arg};

pub fn get_fast_args() -> Option<(String, String)> {
    let matches = App::new("zbb")
        .version("0.1.0")
        .author("Arnaud Gourlay <arnaud.gourlay@gmail.com>")
        .about("The BVG realtime schedules in your terminal")
        .arg(
            Arg::with_name("fast")
                .help("used to by-pass the interactive mode")
                .long("fast")
                .short("F")
                .takes_value(true)
                .value_names(&["STATION", "LINE"])
                .required(false)
                .min_values(2)
                .max_values(2),
        )
        .get_matches();

    let fast_args: Option<(String, String)> = matches.values_of("fast").map(|mut values| {
        (
            values
                .next()
                .expect("impossible because min_values = 2")
                .to_string(),
            values
                .next()
                .expect("impossible because min_values = 2")
                .to_string(),
        )
    });
    fast_args
}
