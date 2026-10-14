//! Content coding of registry responses.
//!
//! Every response goes through a `CompressionLayer` (gzip, zstd) that
//! honours `Accept-Encoding`; handlers may also send a precompressed body
//! themselves (music_metas, see `metas.rs`), which the layer passes through
//! untouched. Bundles (`application/x-tar`) are never compressed here.
//!
//! A content coding is a different representation (RFC 9110 §8.8.3), so a
//! strong ETag must differ between them: `"<tag>"` for identity,
//! `"<tag>-gzip"` / `"<tag>-zstd"` for coded bodies — the same form a Caddy
//! `encode` in front of the registry produced so far. Handlers only ever see
//! and emit the base tag: [`encoded_etags`] strips the suffix from
//! `If-None-Match` (any coding of the same content revalidates it) and adds
//! the suffix to `ETag`, on a 304 for the coding a 200 would have used.

use axum::{
    extract::Request,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::Response,
};
use tower_http::compression::predicate::{NotForContentType, Predicate};
use tower_http::compression::{CompressionLayer, DefaultPredicate};

/// The content codings the registry produces, least to most preferred on a
/// q-value tie (the same order the compression layer uses).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Coding {
    Identity,
    Gzip,
    Zstd,
}

impl Coding {
    pub const COMPRESSED: [Coding; 2] = [Coding::Gzip, Coding::Zstd];

    pub fn token(self) -> &'static str {
        match self {
            Coding::Identity => "identity",
            Coding::Gzip => "gzip",
            Coding::Zstd => "zstd",
        }
    }

    fn parse(token: &str) -> Option<Self> {
        if token.eq_ignore_ascii_case("gzip") || token.eq_ignore_ascii_case("x-gzip") {
            Some(Coding::Gzip)
        } else if token.eq_ignore_ascii_case("zstd") {
            Some(Coding::Zstd)
        } else if token.eq_ignore_ascii_case("identity") {
            Some(Coding::Identity)
        } else {
            None
        }
    }

    fn from_content_encoding(headers: &HeaderMap) -> Option<Self> {
        let value = headers.get(header::CONTENT_ENCODING)?.to_str().ok()?;
        match Coding::parse(value.trim())? {
            Coding::Identity => None,
            coding => Some(coding),
        }
    }
}

/// The coding the compression layer will pick for a request with these
/// headers: highest q-value, ties to the most preferred coding, `*` covering
/// codings not listed, identity when nothing else is acceptable unless the
/// client excluded it (`None`: the layer answers 406). Mirrors
/// tower-http's negotiation; `negotiation_matches_the_compression_layer`
/// checks the two agree.
pub fn negotiate(headers: &HeaderMap) -> Option<Coding> {
    let mut explicit: Vec<(Coding, u16)> = Vec::new();
    let mut wildcard: Option<u16> = None;
    for item in headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
    {
        let mut parts = item.splitn(2, ';');
        let token = parts.next().unwrap_or("").trim();
        let q = match parts.next() {
            Some(q) => match parse_qvalue(q.trim()) {
                Some(q) => q,
                None => continue,
            },
            None => 1000,
        };
        if token == "*" {
            wildcard.get_or_insert(q);
        } else if let Some(coding) = Coding::parse(token) {
            explicit.push((coding, q));
        }
    }
    let best = |candidates: &mut dyn Iterator<Item = (Coding, u16)>| {
        candidates
            .filter(|(_, q)| *q > 0)
            .max_by_key(|&(coding, q)| (q, coding))
            .map(|(coding, _)| coding)
    };
    match wildcard {
        None => {
            let identity_rejected = explicit
                .iter()
                .any(|&(coding, q)| coding == Coding::Identity && q == 0);
            match best(&mut explicit.iter().copied()) {
                Some(coding) => Some(coding),
                None if identity_rejected => None,
                None => Some(Coding::Identity),
            }
        }
        Some(wildcard) => {
            let effective = [Coding::Identity, Coding::Gzip, Coding::Zstd].map(|coding| {
                let q = explicit
                    .iter()
                    .find(|(c, _)| *c == coding)
                    .map(|(_, q)| *q)
                    .unwrap_or(wildcard);
                (coding, q)
            });
            best(&mut effective.into_iter())
        }
    }
}

/// An RFC 9110 q-value (`q=0.5`) as thousandths.
fn parse_qvalue(s: &str) -> Option<u16> {
    let mut chars = s.chars();
    if !matches!(chars.next(), Some('q' | 'Q')) || chars.next() != Some('=') {
        return None;
    }
    let mut value: u16 = match chars.next() {
        Some('0') => 0,
        Some('1') => 1000,
        _ => return None,
    };
    match chars.next() {
        None => return Some(value),
        Some('.') => {}
        _ => return None,
    }
    let mut factor = 100;
    for c in chars {
        let digit = c.to_digit(10)? as u16;
        if factor == 0 {
            return None;
        }
        value += factor * digit;
        factor /= 10;
    }
    (value <= 1000).then_some(value)
}

/// Response compression for the registry router. Tars (the bundle) stay
/// identity: a whole master set compressed per request is a lot of CPU on
/// the registry node, and peers pulling a bundle opt in elsewhere.
pub fn compression_layer() -> CompressionLayer<impl Predicate> {
    CompressionLayer::new().compress_when(
        DefaultPredicate::new()
            .and(NotForContentType::const_new("application/x-tar"))
            .and(NotForContentType::const_new("application/octet-stream")),
    )
}

/// `"<tag>"` -> `"<tag>-<coding>"` (`W/` kept); identity and anything that
/// is not a quoted entity tag stay as they are.
fn with_coding_suffix(etag: &str, coding: Coding) -> Option<String> {
    if coding == Coding::Identity {
        return None;
    }
    let (weak, quoted) = match etag.strip_prefix("W/") {
        Some(rest) => ("W/", rest),
        None => ("", etag),
    };
    let inner = quoted.strip_prefix('"')?.strip_suffix('"')?;
    if strip_coding_suffix(inner) != inner {
        return None;
    }
    Some(format!("{weak}\"{inner}-{}\"", coding.token()))
}

fn strip_coding_suffix(tag: &str) -> &str {
    Coding::COMPRESSED
        .iter()
        .find_map(|coding| {
            tag.strip_suffix(coding.token())
                .and_then(|rest| rest.strip_suffix('-'))
        })
        .unwrap_or(tag)
}

/// `If-None-Match` with every entity tag's coding suffix removed.
fn base_if_none_match(value: &str) -> String {
    value
        .split(',')
        .map(|tag| {
            let tag = tag.trim();
            let (weak, quoted) = match tag.strip_prefix("W/") {
                Some(rest) => ("W/", rest),
                None => ("", tag),
            };
            match quoted.strip_prefix('"').and_then(|t| t.strip_suffix('"')) {
                Some(inner) => format!("{weak}\"{}\"", strip_coding_suffix(inner)),
                None => tag.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Keeps entity tags per representation (see the module docs). Sits outside
/// the compression layer, so it sees the final `Content-Encoding`.
pub async fn encoded_etags(mut request: Request, next: Next) -> Response {
    let negotiated = negotiate(request.headers());
    // Several header lines form one list (RFC 9110 §5.3); handlers read the
    // first line only, so the rewrite joins them.
    let presented: Vec<&str> = request
        .headers()
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect();
    if !presented.is_empty() {
        let value = base_if_none_match(&presented.join(","));
        if let Ok(value) = HeaderValue::from_str(&value) {
            request.headers_mut().insert(header::IF_NONE_MATCH, value);
        }
    }
    let mut response = next.run(request).await;
    let not_modified = response.status() == StatusCode::NOT_MODIFIED;
    let coding = match Coding::from_content_encoding(response.headers()) {
        Some(coding) => Some(coding),
        None if not_modified => negotiated,
        None => None,
    };
    let tagged = coding.and_then(|coding| {
        response
            .headers()
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
            .and_then(|etag| with_coding_suffix(etag, coding))
            .and_then(|etag| HeaderValue::from_str(&etag).ok())
    });
    if let Some(etag) = tagged {
        response.headers_mut().insert(header::ETAG, etag);
    }
    // A 304 carries the `Vary` its 200 would (RFC 9110 §15.4.5); the
    // compression layer adds it only to bodies it considers.
    if not_modified && !vary_accept_encoding(response.headers()) {
        response
            .headers_mut()
            .append(header::VARY, HeaderValue::from_static("accept-encoding"));
    }
    response
}

/// Whether `Vary` already names `Accept-Encoding`.
pub fn vary_accept_encoding(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::VARY)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|v| v.trim().eq_ignore_ascii_case("accept-encoding") || v.trim() == "*")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(accept: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(accept) = accept {
            headers.insert(
                header::ACCEPT_ENCODING,
                HeaderValue::from_str(accept).unwrap(),
            );
        }
        headers
    }

    const CASES: &[Option<&str>] = &[
        None,
        Some(""),
        Some("gzip"),
        Some("x-gzip"),
        Some("GZIP"),
        Some("zstd"),
        Some("gzip, deflate, br, zstd"),
        Some("gzip, zstd;q=0.5"),
        Some("gzip;q=0.5, zstd;q=0.5"),
        Some("gzip;q=0.8, zstd;q=0.9, identity;q=1"),
        Some("br"),
        Some("br, identity;q=0"),
        Some("identity"),
        Some("identity;q=0"),
        Some("gzip;q=0"),
        Some("gzip;q=0, zstd;q=0"),
        Some("*"),
        Some("*;q=0"),
        Some("*;q=0, gzip"),
        Some("*;q=0.5, zstd;q=0"),
        Some("gzip;q=0, *;q=0.2"),
        Some("identity;q=0, *;q=0"),
        Some("gzip;q=1.5"),
        Some("gzip;q=0.0001, identity;q=0"),
        Some("gzip;level=1"),
        Some("gzip ; q=0.5"),
        Some("deflate"),
    ];

    #[test]
    fn negotiation_picks_by_qvalue_then_preference() {
        assert_eq!(negotiate(&headers(None)), Some(Coding::Identity));
        assert_eq!(negotiate(&headers(Some("gzip"))), Some(Coding::Gzip));
        assert_eq!(
            negotiate(&headers(Some("gzip, deflate, br, zstd"))),
            Some(Coding::Zstd)
        );
        assert_eq!(
            negotiate(&headers(Some("gzip, zstd;q=0.5"))),
            Some(Coding::Gzip)
        );
        assert_eq!(negotiate(&headers(Some("identity;q=0"))), None);
        assert_eq!(negotiate(&headers(Some("*;q=0, gzip"))), Some(Coding::Gzip));
    }

    /// The handler-side negotiation (precompressed bodies, 304 ETags) must
    /// pick what the compression layer picks for the same request.
    #[tokio::test]
    async fn negotiation_matches_the_compression_layer() {
        use tower::ServiceExt;
        let app = axum::Router::new()
            .route(
                "/",
                axum::routing::get(|| async {
                    ([("content-type", "application/json")], "[".repeat(256))
                }),
            )
            .layer(compression_layer());
        for accept in CASES {
            let mut request = axum::http::Request::get("/");
            if let Some(accept) = accept {
                request = request.header(header::ACCEPT_ENCODING, *accept);
            }
            let response = app
                .clone()
                .oneshot(request.body(axum::body::Body::empty()).unwrap())
                .await
                .unwrap();
            let layer = if response.status() == StatusCode::NOT_ACCEPTABLE {
                None
            } else {
                Some(Coding::from_content_encoding(response.headers()).unwrap_or(Coding::Identity))
            };
            assert_eq!(
                negotiate(&headers(*accept)),
                layer,
                "Accept-Encoding {accept:?}"
            );
        }
    }

    #[test]
    fn etag_suffixes_round_trip() {
        assert_eq!(
            with_coding_suffix("\"abc\"", Coding::Gzip).as_deref(),
            Some("\"abc-gzip\"")
        );
        assert_eq!(
            with_coding_suffix("W/\"abc\"", Coding::Zstd).as_deref(),
            Some("W/\"abc-zstd\"")
        );
        assert_eq!(with_coding_suffix("\"abc\"", Coding::Identity), None);
        assert_eq!(with_coding_suffix("\"abc-gzip\"", Coding::Gzip), None);
        assert_eq!(with_coding_suffix("abc", Coding::Gzip), None);
        assert_eq!(
            base_if_none_match("\"a-gzip\", W/\"b-zstd\",\"c\", *"),
            "\"a\", W/\"b\", \"c\", *"
        );
        assert_eq!(strip_coding_suffix("abc-6.3.0-zstd"), "abc-6.3.0");
        assert_eq!(strip_coding_suffix("gzip"), "gzip");
    }

    async fn ok_with_etag(headers: HeaderMap) -> Response {
        use axum::response::IntoResponse;
        let tag = "\"d1\"";
        let matched = headers
            .get(header::IF_NONE_MATCH)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(',').any(|t| t.trim() == tag))
            .unwrap_or(false);
        if matched {
            return (StatusCode::NOT_MODIFIED, [("etag", tag)]).into_response();
        }
        (
            [("content-type", "application/json"), ("etag", tag)],
            "{\"k\":\"".to_string() + &"v".repeat(512) + "\"}",
        )
            .into_response()
    }

    #[tokio::test]
    async fn etags_follow_the_coding_and_revalidate_across_codings() {
        use tower::ServiceExt;
        let app = axum::Router::new()
            .route("/", axum::routing::get(ok_with_etag))
            .layer(compression_layer())
            .layer(axum::middleware::from_fn(encoded_etags));
        let call = |accept: Option<&'static str>, inm: Option<&'static str>| {
            let app = app.clone();
            async move {
                let mut request = axum::http::Request::get("/");
                if let Some(accept) = accept {
                    request = request.header(header::ACCEPT_ENCODING, accept);
                }
                if let Some(inm) = inm {
                    request = request.header(header::IF_NONE_MATCH, inm);
                }
                app.oneshot(request.body(axum::body::Body::empty()).unwrap())
                    .await
                    .unwrap()
            }
        };
        let get = |r: &Response, name: header::HeaderName| {
            r.headers()
                .get(name)
                .map(|v| v.to_str().unwrap().to_string())
        };

        let plain = call(None, None).await;
        assert_eq!(get(&plain, header::ETAG).as_deref(), Some("\"d1\""));
        assert_eq!(get(&plain, header::CONTENT_ENCODING), None);
        let gzip = call(Some("gzip"), None).await;
        assert_eq!(
            get(&gzip, header::CONTENT_ENCODING).as_deref(),
            Some("gzip")
        );
        assert_eq!(get(&gzip, header::ETAG).as_deref(), Some("\"d1-gzip\""));
        assert!(vary_accept_encoding(gzip.headers()));
        let zstd = call(Some("gzip, zstd"), None).await;
        assert_eq!(get(&zstd, header::ETAG).as_deref(), Some("\"d1-zstd\""));

        // Revalidation: any coding's tag matches; the 304 carries the tag a
        // 200 to the same request would have.
        let nm = call(Some("gzip"), Some("\"d1-gzip\"")).await;
        assert_eq!(nm.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(get(&nm, header::ETAG).as_deref(), Some("\"d1-gzip\""));
        assert!(vary_accept_encoding(nm.headers()));
        let nm = call(Some("gzip"), Some("\"d1\"")).await;
        assert_eq!(nm.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(get(&nm, header::ETAG).as_deref(), Some("\"d1-gzip\""));
        let nm = call(None, Some("\"d1-gzip\"")).await;
        assert_eq!(nm.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(get(&nm, header::ETAG).as_deref(), Some("\"d1\""));
        let changed = call(Some("gzip"), Some("\"d0-gzip\"")).await;
        assert_eq!(changed.status(), StatusCode::OK);
        let mut request = axum::http::Request::get("/")
            .header(header::IF_NONE_MATCH, "\"d0\"")
            .header(header::IF_NONE_MATCH, "\"d1-zstd\"");
        request = request.header(header::ACCEPT_ENCODING, "zstd");
        let nm = app
            .clone()
            .oneshot(request.body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(nm.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(get(&nm, header::ETAG).as_deref(), Some("\"d1-zstd\""));
    }
}
