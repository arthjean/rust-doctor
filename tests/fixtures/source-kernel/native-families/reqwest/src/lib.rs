//! The surface of reqwest the fixture calls, with none of its behavior.

pub async fn get(_url: &str) -> Result<(), ()> {
    Ok(())
}

pub mod blocking {
    pub fn get(_url: &str) -> Result<(), ()> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct Error;

pub struct StatusCode;

impl StatusCode {
    pub fn is_success(&self) -> bool {
        true
    }
}

pub struct Response;

impl Response {
    pub fn status(&self) -> StatusCode {
        StatusCode
    }

    pub fn error_for_status(self) -> Result<Self, Error> {
        Ok(self)
    }

    pub async fn text(self) -> Result<String, Error> {
        Ok(String::new())
    }

    pub async fn json<T: Default>(self) -> Result<T, Error> {
        Ok(T::default())
    }
}

pub struct RequestBuilder;

impl RequestBuilder {
    pub async fn send(self) -> Result<Response, Error> {
        Ok(Response)
    }
}

pub struct Client;

impl Client {
    pub fn new() -> Self {
        Self
    }

    pub fn get(&self, _url: &str) -> RequestBuilder {
        RequestBuilder
    }
}
