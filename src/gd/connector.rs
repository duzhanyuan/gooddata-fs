#![deny(warnings)]
#![allow(non_snake_case)]
#[allow(unused_imports)]

extern crate time;
extern crate hyper;
extern crate lru_cache;
extern crate tempfile;

use cookie::CookieJar;
use helpers;
use hyper::client::Client;
use hyper::client::response::Response;
use hyper::header::{Accept, Cookie, ContentType, SetCookie, UserAgent, qitem};
use hyper::mime::{Mime, TopLevel, SubLevel, Attr, Value};
use lru_cache::LruCache;
use rest::url;
use rustc_serialize::{json, Encodable, Decodable};
use rustc_serialize::json::DecoderError;
use std::fs::File;
use std::io::{Write, Read, Seek, SeekFrom};


pub struct Connector {
    pub client: Client,
    pub server: String,
    pub jar: CookieJar<'static>,
    pub token_updated: Option<time::PreciseTime>,
    pub cache: LruCache<String, File>,
}

#[allow(dead_code)]
#[allow(unused_variables)]
#[allow(unreachable_code)]
impl Connector {
    pub fn new(server: String, cache_size: usize) -> Connector {
        Connector {
            client: Client::new(),
            server: server,
            jar: CookieJar::new(helpers::random_string(32).as_bytes()),
            token_updated: None,
            cache: LruCache::new(cache_size),
        }
    }

    /// HTTP Method GET Wrapper
    pub fn get<S: Into<String>>(&mut self, path: S) -> Response {
        self.refresh_token_check();

        let uriPath = format!("{}", path.into());
        let uri = format!("{}{}", self.server, uriPath);
        let raw = self.client
            .get(&uri[..])
            .header(ContentType(Mime(TopLevel::Application,
                                     SubLevel::Json,
                                     vec![(Attr::Charset, Value::Utf8)])))
            .header(Accept(vec![
                             qitem(Mime(TopLevel::Application, SubLevel::Json,
                             vec![(Attr::Charset, Value::Utf8)])),
            ]))
            .header(UserAgent(Connector::user_agent().to_owned()))
            .header(Cookie::from_cookie_jar(&self.jar))
            .send();

        info!("GoodDataClient::get() - Response: {:?}", raw);
        if !raw.is_ok() {
            return self.get(uriPath);
        }

        let mut res = raw.unwrap();

        // assert_eq!(res.status, hyper::Ok);
        if res.status != hyper::Ok {
            return res;
        }

        self.print_response(&mut res);
        self.update_cookie_jar(&res);

        // self.cache.insert(uriPath, res);

        return res;
    }

    /// HTTP Method GET Wrapper
    pub fn get_cached<S: Into<String>>(&mut self, path: S, force_update: bool) -> String {
        let key: String = format!("{}", path.into());
        if !force_update && self.cache.contains_key(&key) {
            let mut file: &File = self.cache.get_mut(&key).unwrap();

            info!("get_cached() - Reading {:?}", file);

            // Seek to beginning of file
            file.seek(SeekFrom::Start(0)).unwrap();

            // Read content of temporary file
            let mut buf = String::new();
            file.read_to_string(&mut buf).unwrap();

            return buf;
        }

        let mut res = self.get(key.clone());
        let raw = self.get_content(&mut res);

        let mut file: File = tempfile::tempfile().unwrap();
        write!(file, "{}", raw).unwrap();

        info!("get_cached() - Creating {:?} - Content: {:?}",
              file,
              raw.clone());

        self.cache.insert(key.clone(), file);
        return raw.clone();
    }

    pub fn object_by_get<TypeTo: Decodable>(&mut self, link: String) -> Option<TypeTo> {
        let mut res = self.get(link);

        if res.status != hyper::Ok {
            return None;
        }

        let raw = self.get_content(&mut res);
        let obj: Result<TypeTo, DecoderError> = json::decode(&raw.to_string());
        match obj {
            Ok(obj) => Some(obj),
            Err(e) => None,
        }
    }

    pub fn object_by_post<TypeFrom: Encodable, TypeTo: Decodable>(&mut self,
                                                                  link: String,
                                                                  payload: TypeFrom)
                                                                  -> Option<TypeTo> {
        let mut res = self.post(link, json::encode(&payload).unwrap());
        let raw = self.get_content(&mut res);

        if ![hyper::Ok, hyper::status::StatusCode::Created].contains(&res.status) {
            error!("failed post content: {}", &raw);
            return None;
        }

        let obj: Result<TypeTo, DecoderError> = json::decode(&raw.to_string());
        match obj {
            Ok(obj) => Some(obj),
            Err(e) => {
                error!("GoodData::Connector::object_by_post() - {:?}", e);
                None
            }
        }
    }


    /// HTTP Method POST Wrapper
    pub fn post<S: Into<String>>(&mut self, path: S, body: S) -> hyper::client::response::Response {
        self.refresh_token_check();

        let uriPath = format!("{}", path.into());
        let uri = format!("{}{}", self.server, uriPath);
        let payload = body.into();

        let raw = self.client
            .post(&uri[..])
            .header(ContentType(Mime(TopLevel::Application,
                                     SubLevel::Json,
                                     vec![(Attr::Charset, Value::Utf8)])))
            .header(UserAgent(Connector::user_agent().to_owned()))
            .header(Accept(vec![
                            qitem(Mime(TopLevel::Application, SubLevel::Json,
                            vec![(Attr::Charset, Value::Utf8)])),
            ]))
            .header(Cookie::from_cookie_jar(&self.jar))
            .body(&payload[..])
            .send();


        match raw {
            Err(e) => {
                error!("GoodData::connector::port() - {:?}", e);
                return self.post(uriPath, payload);
            }
            _ => {}
        }

        let mut res = raw.unwrap();
        assert!([hyper::Ok, hyper::status::StatusCode::Created].contains(&res.status));

        self.print_response(&mut res);
        self.update_cookie_jar(&res);

        return res;
    }

    pub fn delete<S: Into<String>>(&mut self, path: S) -> hyper::client::response::Response {
        self.refresh_token_check();

        let uriPath = format!("{}", path.into());
        let uri = format!("{}{}", self.server, uriPath);

        let raw = self.client
            .delete(&uri[..])
            .header(UserAgent(Connector::user_agent().to_owned()))
            .header(Accept(vec![qitem(Mime(TopLevel::Application,
                                           SubLevel::Json,
                                           vec![(Attr::Charset, Value::Utf8)]))]))
            .header(Cookie::from_cookie_jar(&self.jar))
            .send();

        info!("GoodDataClient::delete() - Response: {:?}", raw);

        let mut res = raw.unwrap();
        assert_eq!(res.status, hyper::Ok);

        self.print_response(&mut res);
        self.update_cookie_jar(&res);

        return res;
    }

    /// Get HTTP Response body
    pub fn get_content(&mut self, res: &mut hyper::client::Response) -> String {
        let mut buf = String::new();
        debug!("{:?}", res.read_to_string(&mut buf));
        match res.read_to_string(&mut buf) {
            Ok(_) => (),
            Err(_) => panic!("I give up."),
        };

        return buf;
    }

    /// Print HTTP Response
    pub fn print_response(&mut self, res: &mut hyper::client::Response) {
        return;

        let obj = res;

        debug!("{:?}", obj);

        let content = self.get_content(obj);
        debug!("{}", content);
    }

    /// Update Cookies in Jar from HTTP Response
    fn update_cookie_jar(&mut self, res: &hyper::client::Response) {
        for setCookie in res.headers.get::<SetCookie>().iter() {
            for cookie in setCookie.iter() {
                self.jar.add(cookie.clone());
            }
        }
    }

    /// Refresh GoodData TT (Temporary Token)
    pub fn refresh_token(&mut self) {
        // Refresh token
        // self.get("/gdc/account/token");

        let uri = format!("{}{}", self.server, url::TOKEN);
        let raw = self.client
            .get(&uri[..])
            .header(ContentType(Mime(TopLevel::Application,
                                     SubLevel::Json,
                                     vec![(Attr::Charset, Value::Utf8)])))
            .header(Accept(vec![
                             qitem(Mime(TopLevel::Application, SubLevel::Json,
                             vec![(Attr::Charset, Value::Utf8)])),
            ]))
            .header(UserAgent(Connector::user_agent().to_owned()))
            .header(Cookie::from_cookie_jar(&self.jar))
            .send();

        debug!("{:?}", raw);
        if !raw.is_ok() {
            return self.refresh_token();
        }

        let mut res = raw.unwrap();
        assert_eq!(res.status, hyper::Ok);
        debug!("{:?}", res);

        self.print_response(&mut res);
        self.update_cookie_jar(&res);

        self.token_updated = Some(time::PreciseTime::now());
    }

    fn refresh_token_check(&mut self) {
        if self.token_updated.is_some() &&
           self.token_updated.unwrap().to(time::PreciseTime::now()) >
           time::Duration::seconds(4 * 60) {
            self.refresh_token();
        }
    }

    /// Construct User-Agent HTTP Header
    fn user_agent() -> String {
        const VERSION: &'static str = env!("CARGO_PKG_VERSION");
        return format!("gooddata-fs/{}", VERSION);
    }
}
