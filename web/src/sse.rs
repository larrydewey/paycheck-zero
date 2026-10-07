//! Datastar 1.0 server-sent events (spec §7.4): the backend drives the UI by
//! streaming `datastar-patch-elements` / `datastar-patch-signals` events.

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, SET_COOKIE};
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Response};
use maud::{html, Markup};

#[derive(Default)]
pub struct Sse {
    body: String,
    cookies: Vec<HeaderValue>,
}

impl Sse {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn event(&mut self, name: &str, lines: &[(&str, &str)]) {
        self.body.push_str("event: ");
        self.body.push_str(name);
        self.body.push('\n');
        for (key, value) in lines {
            for line in value.split('\n') {
                self.body.push_str("data: ");
                self.body.push_str(key);
                self.body.push(' ');
                self.body.push_str(line.trim_end_matches('\r'));
                self.body.push('\n');
            }
        }
        self.body.push('\n');
    }

    /// Morphs elements into the page by their `id` (outer morph).
    #[must_use]
    pub fn patch(mut self, markup: Markup) -> Self {
        self.event("datastar-patch-elements", &[("elements", &markup.into_string())]);
        self
    }

    /// Patches with an explicit selector and mode (`inner`, `append`, …).
    #[must_use]
    pub fn patch_into(mut self, selector: &str, mode: &str, markup: Markup) -> Self {
        self.event(
            "datastar-patch-elements",
            &[("selector", selector), ("mode", mode), ("elements", &markup.into_string())],
        );
        self
    }

    /// Adds another response's events after this one's.
    #[must_use]
    pub fn append(mut self, other: Sse) -> Self {
        self.body.push_str(&other.body);
        self.cookies.extend(other.cookies);
        self
    }

    /// Removes elements matching a selector.
    #[must_use]
    pub fn remove(mut self, selector: &str) -> Self {
        self.event("datastar-patch-elements", &[("selector", selector), ("mode", "remove")]);
        self
    }

    /// Merges signals (JSON object).
    #[must_use]
    pub fn signals(mut self, json: &serde_json::Value) -> Self {
        self.event("datastar-patch-signals", &[("signals", &json.to_string())]);
        self
    }

    /// Runs a script in the browser, then removes the script element.
    #[must_use]
    pub fn script(self, js: &str) -> Self {
        self.patch_into(
            "body",
            "append",
            html! { script data-effect="el.remove()" { (maud::PreEscaped(js)) } },
        )
    }

    /// Navigates the browser to `url`.
    #[must_use]
    pub fn redirect(self, url: &str) -> Self {
        let js = format!("window.location.assign({})", serde_json::to_string(url).unwrap_or_else(|_| "\"/\"".into()));
        self.script(&js)
    }

    #[must_use]
    pub fn with_cookies(mut self, cookies: Vec<HeaderValue>) -> Self {
        self.cookies.extend(cookies);
        self
    }
}

impl IntoResponse for Sse {
    fn into_response(self) -> Response {
        let mut resp = self.body.into_response();
        let h = resp.headers_mut();
        h.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
        h.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        for c in self.cookies {
            h.append(SET_COOKIE, c);
        }
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_line_elements_are_prefixed() {
        let s = Sse::new().patch(html! { div id="x" { (maud::PreEscaped("a\nb")) } });
        assert_eq!(s.body, "event: datastar-patch-elements\ndata: elements <div id=\"x\">a\ndata: elements b</div>\n\n");
    }
}
