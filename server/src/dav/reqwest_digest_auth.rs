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

        // Answer the scheme the server asks for. Digest is preferred when a
        // server offers both; Basic covers Radicale, Nextcloud and most
        // hosted DAV servers, which only speak Basic.
        let challenges: Vec<String> = resp
            .headers()
            .get_all("www-authenticate")
            .iter()
            .filter_map(|v| v.to_str().ok().map(|s| s.trim().to_string()))
            .collect();
        let digest = challenges.iter().find(|s| s.to_lowercase().starts_with("digest"));
        let basic = challenges.iter().any(|s| s.to_lowercase().starts_with("basic"));
        let auth_header = match (digest, basic) {
            (Some(d), _) => d.clone(),
            (None, true) => {
                return cloned
                    .basic_auth(&self.username, Some(&self.password))
                    .send()
                    .await;
            }
            (None, false) => return Ok(resp),
        };

        let mut parsed = match parse_digest(&auth_header) {
            Ok(p) => p,
            Err(_) => return Ok(resp),
        };

        // Use the actual HTTP method in the digest hash calculation —
        // AuthContext::new defaults to GET, which would produce an invalid
        // response for PROPFIND/REPORT/PUT requests.
        let method_str = self.method.as_str();
        let context = AuthContext::new_with_method(
            &self.username,
            &self.password,
            &self.url,
            None::<Vec<u8>>,
            digest_auth::HttpMethod(std::borrow::Cow::Borrowed(method_str)),
        );
        let answer = match parsed.respond(&context) {
            Ok(a) => a.to_string(),
            Err(_) => return Ok(resp),
        };

        cloned.header("Authorization", &answer).send().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A server that asks for Basic once, then answers 207 and hands back
    /// the Authorization header of the second request.
    async fn basic_server() -> (String, tokio::task::JoinHandle<Option<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/dav/", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            let mut seen = None;
            for answer in [
                "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"dav\"\r\nContent-Length: 0\r\n\r\n",
                "HTTP/1.1 207 Multi-Status\r\nContent-Length: 0\r\n\r\n",
            ] {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                seen = req
                    .lines()
                    .find(|l| l.to_lowercase().starts_with("authorization:"))
                    .map(|l| l["authorization:".len()..].trim().to_string());
                sock.write_all(answer.as_bytes()).await.unwrap();
            }
            seen
        });
        (url, handle)
    }

    #[tokio::test]
    async fn answers_a_basic_challenge() {
        let (url, server) = basic_server().await;
        let client = ClientBuilder::new(ReqwestClient::new())
            .username("erika".into())
            .password("geheim".into())
            .build();
        let resp = client
            .request(Method::from_bytes(b"PROPFIND").unwrap(), &url)
            .body("<propfind/>")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 207);
        // base64("erika:geheim")
        assert_eq!(server.await.unwrap().as_deref(), Some("Basic ZXJpa2E6Z2VoZWlt"));
    }
}
