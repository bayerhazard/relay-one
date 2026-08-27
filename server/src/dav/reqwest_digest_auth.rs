use digest_auth::{AuthContext, parse as parse_digest};
use reqwest::{Client as ReqwestClient, Method, Response};

pub struct ClientBuilder {
    inner: ReqwestClient,
    username: String,
    password: String,
}

impl ClientBuilder {
    pub fn new(inner: ReqwestClient) -> Self {
        Self { inner, username: String::new(), password: String::new() }
    }

    pub fn username(mut self, username: String) -> Self {
        self.username = username;
        self
    }

    pub fn password(mut self, password: String) -> Self {
        self.password = password;
        self
    }

    pub fn build(self) -> Client {
        Client {
            inner: self.inner,
            username: self.username,
            password: self.password,
        }
    }
}

pub struct Client {
    inner: ReqwestClient,
    username: String,
    password: String,
}

impl Client {
    pub fn get(&self, url: &str) -> RequestBuilder {
        RequestBuilder {
            inner: self.inner.get(url),
            method: Method::GET,
            url: url.to_string(),
            username: self.username.clone(),
            password: self.password.clone(),
        }
    }

    pub fn request(&self, method: Method, url: &str) -> RequestBuilder {
        RequestBuilder {
            inner: self.inner.request(method.clone(), url),
            method,
            url: url.to_string(),
            username: self.username.clone(),
            password: self.password.clone(),
        }
    }
}

pub struct RequestBuilder {
    inner: reqwest::RequestBuilder,
    #[allow(dead_code)]
    method: Method,
    url: String,
    username: String,
    password: String,
}

impl RequestBuilder {
    pub fn header(mut self, key: &str, value: &str) -> Self {
        self.inner = self.inner.header(key, value);
        self
    }

    pub fn body(mut self, body: impl Into<reqwest::Body>) -> Self {
        self.inner = self.inner.body(body);
        self
    }

    pub async fn send(self) -> Result<Response, reqwest::Error> {
        // Clone the request so we can retry with digest auth if needed
        let cloned = match self.inner.try_clone() {
            Some(c) => c,
            None => return self.inner.send().await,
        };

        let resp = self.inner.send().await?;

        if resp.status() != 401 {
            return Ok(resp);
        }

        let auth_header = match resp.headers().get("www-authenticate") {
            Some(v) => match v.to_str() {
                Ok(s) if s.trim().to_lowercase().starts_with("digest") => s.to_string(),
                _ => return Ok(resp),
            },
            None => return Ok(resp),
        };

        let mut parsed = match parse_digest(&auth_header) {
            Ok(p) => p,
            Err(_) => return Ok(resp),
        };

        let context = AuthContext::new(&self.username, &self.password, &self.url);
        let answer = match parsed.respond(&context) {
            Ok(a) => a.to_string(),
            Err(_) => return Ok(resp),
        };

        cloned.header("Authorization", &answer).send().await
    }
}
