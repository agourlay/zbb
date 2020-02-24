#[derive(Debug)]
pub enum ZbbError {
    StdIoError { e: std::io::Error },
    NetworkError { message: String }
}

impl std::convert::From<reqwest::Error> for ZbbError {
    fn from(e: reqwest::Error) -> Self {
            ZbbError::NetworkError {
                message: e.to_string(),
        }
    }
}

impl std::convert::From<std::io::Error> for ZbbError {
    fn from(e: std::io::Error) -> Self {
        ZbbError::StdIoError { e }
    }
}