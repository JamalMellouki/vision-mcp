//! Capture a client-rendered page through the Chrome DevTools Protocol.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;
use vision::{Point, Resource, Snapshot, Stylesheet, Viewport};

use crate::browser::{BrowserProcess, executable};
use crate::cdp::Cdp;
use crate::error::{Error, Result};

const BASE64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;
const MAX_RESOURCE_BYTES: usize = 32 * 1024 * 1024;

const PAGE_STATE: &str = r#"(() => {
  const clone = document.documentElement.cloneNode(true);
  const originals = document.querySelectorAll('input, textarea, select');
  const copies = clone.querySelectorAll('input, textarea, select');
  for (let index = 0; index < originals.length && index < copies.length; index += 1) {
    const source = originals[index];
    const target = copies[index];
    if (source.tagName === 'TEXTAREA') {
      target.textContent = source.value;
    } else if (source.tagName === 'SELECT') {
      for (let option = 0; option < source.options.length; option += 1) {
        if (source.options[option].selected) {
          target.options[option].setAttribute('selected', '');
        } else {
          target.options[option].removeAttribute('selected');
        }
      }
    } else if (source.type === 'checkbox' || source.type === 'radio') {
      if (source.checked) {
        target.setAttribute('checked', '');
      } else {
        target.removeAttribute('checked');
      }
    } else {
      target.setAttribute('value', source.value);
    }
  }
  return JSON.stringify({
    html: clone.outerHTML,
    links: Array.from(document.querySelectorAll('link[rel~="stylesheet"]')).map(link => ({
      href: link.getAttribute('href') || '',
      absolute: link.href
    })),
    images: Array.from(document.querySelectorAll('img[src]')).map(image => ({
      src: image.getAttribute('src') || '',
      absolute: image.src
    })),
    viewport: {
      width: window.innerWidth,
      height: window.innerHeight,
      dpr: window.devicePixelRatio,
      scrollX: window.scrollX,
      scrollY: window.scrollY
    }
  });
})()"#;

/// How long to wait after navigation.
#[derive(Clone, Debug)]
pub enum WaitStrategy {
    /// Wait for the page load event.
    Load,
    /// Wait for the load event and a quiet network period.
    NetworkIdle {
        /// How long the network must stay quiet.
        quiet: Duration,
    },
    /// Wait until a selector matches.
    Selector(String),
    /// Wait until `document.fonts.ready` settles.
    FontsReady,
}

impl Default for WaitStrategy {
    fn default() -> Self {
        Self::NetworkIdle {
            quiet: Duration::from_millis(500),
        }
    }
}

/// Options for a browser capture.
#[derive(Clone, Debug)]
pub struct CaptureOptions {
    /// The HTTP or HTTPS URL to open.
    pub url: String,
    /// The CSS viewport and device pixel ratio.
    pub viewport: Viewport,
    /// The point at which the page is considered ready.
    pub wait: WaitStrategy,
    /// Overall timeout for launch, navigation, and resource collection.
    pub timeout: Duration,
    /// Allow a URL that is not localhost.
    pub allow_remote: bool,
    /// Explicit Chromium executable.
    pub executable: Option<PathBuf>,
    /// Pass `--no-sandbox` to Chromium. Use only inside a disposable container.
    pub no_sandbox: bool,
    /// Optional user agent.
    pub user_agent: Option<String>,
}

impl CaptureOptions {
    /// Creates options with a localhost-friendly default.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            viewport: Viewport::default(),
            wait: WaitStrategy::default(),
            timeout: Duration::from_secs(30),
            allow_remote: false,
            executable: None,
            no_sandbox: false,
            user_agent: None,
        }
    }
}

/// Captures a page and returns a self-contained snapshot.
pub fn capture(options: &CaptureOptions) -> Result<Snapshot> {
    let url = Url::parse(&options.url)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::DisallowedUrl(format!(
            "{} is not an HTTP URL",
            options.url
        )));
    }
    if !options.allow_remote && !is_local(&url) {
        return Err(Error::DisallowedUrl(format!(
            "{} is remote; pass allow_remote to capture it",
            options.url
        )));
    }
    if options.viewport.width == 0 || options.viewport.height == 0 {
        return Err(Error::DisallowedUrl(
            "viewport dimensions must be greater than zero".to_owned(),
        ));
    }
    if !options.viewport.dpr.is_finite() || options.viewport.dpr <= 0.0 {
        return Err(Error::DisallowedUrl(
            "device pixel ratio must be a positive number".to_owned(),
        ));
    }

    let executable = executable(options.executable.as_deref())?;
    let mut browser = BrowserProcess::launch(&executable, options.no_sandbox)?;
    let websocket = browser.websocket_url(options.timeout)?;
    let mut cdp = Cdp::connect(&websocket)?;

    let target = cdp.command(None, "Target.createTarget", json!({"url": "about:blank"}))?;
    let target_id = required_string(&target, "targetId")?;
    let attached = cdp.command(
        None,
        "Target.attachToTarget",
        json!({"targetId": target_id, "flatten": true}),
    )?;
    let session = required_string(&attached, "sessionId")?;

    cdp.command(Some(&session), "Page.enable", json!({}))?;
    cdp.command(
        Some(&session),
        "Network.enable",
        json!({
            "maxTotalBufferSize": 100_000_000,
            "maxResourceBufferSize": 32_000_000
        }),
    )?;
    cdp.command(
        Some(&session),
        "Network.setCacheDisabled",
        json!({"cacheDisabled": true}),
    )?;
    cdp.command(Some(&session), "Runtime.enable", json!({}))?;
    cdp.command(
        Some(&session),
        "Emulation.setDeviceMetricsOverride",
        json!({
            "width": options.viewport.width,
            "height": options.viewport.height,
            "deviceScaleFactor": options.viewport.dpr,
            "mobile": false
        }),
    )?;
    if let Some(user_agent) = &options.user_agent {
        cdp.command(
            Some(&session),
            "Network.setUserAgentOverride",
            json!({"userAgent": user_agent}),
        )?;
    }

    let mut watch = NetworkWatch::new();
    let navigation = cdp.command(
        Some(&session),
        "Page.navigate",
        json!({"url": url.as_str()}),
    )?;
    if let Some(error) = navigation.get("errorText").and_then(Value::as_str) {
        if !error.is_empty() {
            return Err(Error::Navigation(error.to_owned()));
        }
    }
    drain(&mut cdp, &mut watch)?;
    wait_for(&mut cdp, &session, &mut watch, options)?;

    let state_value = evaluate(&mut cdp, &session, PAGE_STATE, false)?;
    let Some(state_json) = state_value.as_str() else {
        return Err(Error::Data("page state is not a string".to_owned()));
    };
    let state: PageState = serde_json::from_str(state_json)?;
    drain(&mut cdp, &mut watch)?;

    let needed = state
        .links
        .iter()
        .map(|link| link.absolute.clone())
        .chain(state.images.iter().map(|image| image.absolute.clone()))
        .collect::<HashSet<_>>();
    let requested = watch
        .resources
        .iter()
        .filter(|(_, resource)| {
            needed.contains(&resource.url)
                && resource.body.is_none()
                && (resource.mime.starts_with("text/css") || resource.mime.starts_with("image/"))
        })
        .map(|(request_id, resource)| (request_id.clone(), resource.url.clone()))
        .collect::<Vec<_>>();
    for (request_id, _) in requested {
        let response = cdp.command(
            Some(&session),
            "Network.getResponseBody",
            json!({"requestId": request_id}),
        )?;
        drain(&mut cdp, &mut watch)?;
        let Some(body) = response.get("body").and_then(Value::as_str) else {
            continue;
        };
        let bytes = if response
            .get("base64Encoded")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            BASE64
                .decode(body)
                .map_err(|error| Error::Data(error.to_string()))?
        } else {
            body.as_bytes().to_vec()
        };
        if bytes.len() > MAX_RESOURCE_BYTES {
            continue;
        }
        if let Some(resource) = watch.resources.get_mut(&request_id) {
            resource.body = Some(bytes);
        }
    }

    let mut snapshot = Snapshot::from_html(state.html).with_viewport(
        Viewport::new(state.viewport.width.max(1), state.viewport.height.max(1))
            .with_dpr(state.viewport.dpr.max(0.1)),
    );
    snapshot.state.scroll = Point {
        x: state.viewport.scroll_x,
        y: state.viewport.scroll_y,
    };

    let mut seen_links = HashSet::new();
    for link in state.links {
        if !seen_links.insert(link.href.clone()) {
            continue;
        }
        let Some(resource) = watch
            .resources
            .values()
            .find(|resource| resource.url == link.absolute)
        else {
            continue;
        };
        let Some(body) = &resource.body else {
            continue;
        };
        snapshot.stylesheets.push(Stylesheet {
            href: link.href,
            css: String::from_utf8_lossy(body).into_owned(),
        });
    }

    let mut seen_images = HashSet::new();
    for image in state.images {
        if !seen_images.insert(image.src.clone()) {
            continue;
        }
        let Some(resource) = watch
            .resources
            .values()
            .find(|resource| resource.url == image.absolute)
        else {
            continue;
        };
        let Some(body) = &resource.body else {
            continue;
        };
        snapshot.resources.push(Resource {
            url: image.src,
            bytes: body.clone(),
        });
    }

    let _ = cdp.command(None, "Target.closeTarget", json!({"targetId": target_id}));
    drop(cdp);
    drop(browser);
    Ok(snapshot)
}

fn is_local(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "::1" | "[::1]")
    )
}

fn required_string(value: &Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Error::Data(format!("missing {key}")))
}

fn evaluate(cdp: &mut Cdp, session: &str, expression: &str, await_promise: bool) -> Result<Value> {
    let response = cdp.command(
        Some(session),
        "Runtime.evaluate",
        json!({
            "expression": expression,
            "returnByValue": true,
            "awaitPromise": await_promise
        }),
    )?;
    if response.get("exceptionDetails").is_some() {
        return Err(Error::Data(
            response
                .get("exceptionDetails")
                .and_then(|details| details.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("script exception")
                .to_owned(),
        ));
    }
    Ok(response
        .get("result")
        .and_then(|result| result.get("value"))
        .cloned()
        .unwrap_or(Value::Null))
}

fn drain(cdp: &mut Cdp, watch: &mut NetworkWatch) -> Result<()> {
    for event in cdp.drain_events() {
        watch.handle(&event)?;
    }
    Ok(())
}

fn wait_for(
    cdp: &mut Cdp,
    session: &str,
    watch: &mut NetworkWatch,
    options: &CaptureOptions,
) -> Result<()> {
    let deadline = Instant::now() + options.timeout;
    let mut last_selector_check = Instant::now() - Duration::from_secs(1);
    loop {
        let ready = match &options.wait {
            WaitStrategy::Load => watch.loaded,
            WaitStrategy::NetworkIdle { quiet } => {
                watch.loaded && watch.inflight.is_empty() && watch.last_activity.elapsed() >= *quiet
            }
            WaitStrategy::Selector(_) => false,
            WaitStrategy::FontsReady => false,
        };
        if ready {
            return Ok(());
        }
        if matches!(options.wait, WaitStrategy::FontsReady) && watch.loaded {
            // `evaluate` passes awaitPromise=true, so CDP does not answer until
            // the font-loading promise settles. Check the resolved value as well
            // instead of returning on the first successful CDP call.
            let settled = evaluate(cdp, session, "document.fonts.ready.then(() => true)", true)?;
            if settled.as_bool() == Some(true) {
                drain(cdp, watch)?;
                return Ok(());
            }
        }
        if let WaitStrategy::Selector(selector) = &options.wait {
            if last_selector_check.elapsed() >= Duration::from_millis(100) {
                let expression = format!(
                    "document.querySelector({}) !== null",
                    serde_json::to_string(selector)?
                );
                if evaluate(cdp, session, &expression, false)?
                    .as_bool()
                    .unwrap_or(false)
                {
                    return Ok(());
                }
                last_selector_check = Instant::now();
            }
        }
        if Instant::now() >= deadline {
            return Err(Error::Timeout(format!("waiting for {:?}", options.wait)));
        }
        if let Some(event) = cdp.next_event(Duration::from_millis(100))? {
            watch.handle(&event)?;
        }
    }
}

struct NetworkWatch {
    loaded: bool,
    inflight: HashSet<String>,
    resources: HashMap<String, ResourceRecord>,
    last_activity: Instant,
}

impl NetworkWatch {
    fn new() -> Self {
        Self {
            loaded: false,
            inflight: HashSet::new(),
            resources: HashMap::new(),
            last_activity: Instant::now(),
        }
    }

    fn handle(&mut self, event: &Value) -> Result<()> {
        let method = event
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = event.get("params").cloned().unwrap_or(Value::Null);
        match method {
            "Page.loadEventFired" => self.loaded = true,
            "Inspector.targetCrashed" => {
                return Err(Error::Protocol("the page crashed".to_owned()));
            }
            "Network.requestWillBeSent" => {
                if let Some(request_id) = params.get("requestId").and_then(Value::as_str) {
                    self.inflight.insert(request_id.to_owned());
                }
                self.last_activity = Instant::now();
            }
            "Network.responseReceived" => {
                let Some(request_id) = params.get("requestId").and_then(Value::as_str) else {
                    return Ok(());
                };
                let response = params.get("response").cloned().unwrap_or(Value::Null);
                let record = ResourceRecord {
                    url: response
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    mime: response
                        .get("mimeType")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    body: None,
                };
                self.resources.insert(request_id.to_owned(), record);
                self.last_activity = Instant::now();
            }
            "Network.loadingFinished" | "Network.loadingFailed" => {
                if let Some(request_id) = params.get("requestId").and_then(Value::as_str) {
                    self.inflight.remove(request_id);
                }
                self.last_activity = Instant::now();
            }
            _ => {}
        }
        Ok(())
    }
}

struct ResourceRecord {
    url: String,
    mime: String,
    body: Option<Vec<u8>>,
}

#[derive(Deserialize)]
struct PageState {
    html: String,
    links: Vec<PageLink>,
    images: Vec<PageImage>,
    viewport: PageViewport,
}

#[derive(Deserialize)]
struct PageLink {
    href: String,
    absolute: String,
}

#[derive(Deserialize)]
struct PageImage {
    src: String,
    absolute: String,
}

#[derive(Deserialize)]
struct PageViewport {
    width: u32,
    height: u32,
    dpr: f32,
    #[serde(rename = "scrollX")]
    scroll_x: f32,
    #[serde(rename = "scrollY")]
    scroll_y: f32,
}
