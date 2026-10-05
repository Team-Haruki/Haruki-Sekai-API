use crate::config::ServerRegion;
use serde_json::Value;

pub(crate) const THUMBNAIL_PREFIX: &str = "/image/mysekai-housing-competition/thumbnail/";

pub(crate) fn thumbnail_origin(region: ServerRegion) -> Option<&'static str> {
    match region {
        ServerRegion::Cn => Some("https://mk-prod-tos.tos-cn-shanghai.volces.com"),
        ServerRegion::Tw => Some("https://mkoversea-prod-bucket.s3.ap-northeast-1.amazonaws.com"),
        ServerRegion::Kr => Some("https://mkkorea-prod-bucket.s3.ap-northeast-1.amazonaws.com"),
        _ => None,
    }
}

pub(crate) fn valid_thumbnail_path(path: &str) -> bool {
    let Some((hash, id)) = path.split_once('/') else {
        return false;
    };
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && id.len() == 36
        && uuid::Uuid::parse_str(id).is_ok()
}

/// Keep the public housing response contract identical across game publishers.
/// Unknown URLs remain untouched rather than silently pointing at another object.
pub(crate) fn normalize_thumbnails(region: ServerRegion, body: &mut Value) {
    let Some(origin) = thumbnail_origin(region) else {
        return;
    };
    normalize_value(&format!("{origin}{THUMBNAIL_PREFIX}"), body);
}

fn normalize_value(prefix: &str, value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if matches!(key.as_str(), "thumbnailPath" | "thumbnail_path") {
                    if let Some(path) = value.as_str().and_then(|s| s.strip_prefix(prefix)) {
                        if valid_thumbnail_path(path) {
                            *value = Value::String(path.to_owned());
                        }
                    }
                } else {
                    normalize_value(prefix, value);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize_value(prefix, item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn path() -> String {
        format!("{}/12345678-1234-1234-1234-123456789abc", "a".repeat(64))
    }

    #[test]
    fn normalizes_all_nuverse_regions_and_nested_housing_responses() {
        for region in [ServerRegion::Cn, ServerRegion::Tw, ServerRegion::Kr] {
            let url = format!(
                "{}{}{}",
                thumbnail_origin(region).unwrap(),
                THUMBNAIL_PREFIX,
                path()
            );
            let mut body = json!({"results": [{"entries": [{"thumbnailPath":url,"reviewCount":42}]}],
                "thumbnail_path":url,"name":url,"relative":{"thumbnailPath":path()}});
            normalize_thumbnails(region, &mut body);
            assert_eq!(body["results"][0]["entries"][0]["thumbnailPath"], path());
            assert_eq!(body["results"][0]["entries"][0]["reviewCount"], 42);
            assert_eq!(body["thumbnail_path"], path());
            assert_eq!(body["relative"]["thumbnailPath"], path());
            assert_eq!(body["name"], url);
            let normalized = body.clone();
            normalize_thumbnails(region, &mut body);
            assert_eq!(body, normalized);
        }
    }

    #[test]
    fn preserves_cp_responses_and_unknown_or_malformed_urls() {
        let base = format!(
            "{}{}",
            thumbnail_origin(ServerRegion::Cn).unwrap(),
            THUMBNAIL_PREFIX
        );
        for region in [ServerRegion::Jp, ServerRegion::En] {
            let mut body = json!({"thumbnailPath":path(),"other":{"thumbnailPath":format!("{base}{}",path())}});
            let original = body.clone();
            normalize_thumbnails(region, &mut body);
            assert_eq!(body, original);
        }
        for address in [
            format!("{base}bad"),
            format!("{base}{}?x=1", path()),
            format!("{base}{}#x", path()),
            format!("https://example.com{THUMBNAIL_PREFIX}{}", path()),
        ] {
            let mut body = json!({"thumbnailPath":address});
            normalize_thumbnails(ServerRegion::Cn, &mut body);
            assert_eq!(body["thumbnailPath"], address);
        }
        for invalid in [
            "bad".to_string(),
            format!("{}/../etc/passwd", "a".repeat(64)),
            format!("{}/{}", "G".repeat(64), "a".repeat(36)),
        ] {
            assert!(!valid_thumbnail_path(&invalid));
        }
    }
}
