//! A small, in-process JavaScript runtime with a DOM facade.
//!
//! QuickJS executes trusted local scripts. The DOM and browser APIs are kept
//! deliberately small: this is not a sandbox and must not be used for
//! untrusted public pages.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use rquickjs::{
    Array, Context, Ctx, Function, Object, Result as JsResult, Runtime, Value, function::Rest,
    object::Accessor,
};

use crate::compat::CompatReport;
use crate::document::selector::parse_selector_list;
use crate::document::{Document, NodeId, NodeKind};
use crate::snapshot::Snapshot;

pub(crate) type SharedDocument = Rc<RefCell<Document>>;

/// Resource limits for one embedded JavaScript runtime.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ScriptLimits {
    pub(crate) timeout: Duration,
    pub(crate) memory_bytes: usize,
    pub(crate) stack_bytes: usize,
}

/// A retained JavaScript context for one prepared page.
///
/// The context owns the live DOM shared by every wrapper created during script
/// execution. Sessions keep this object alive so registered event listeners can
/// run after the initial layout.
pub(crate) struct ScriptRuntime {
    context: Context,
    _runtime: Runtime,
    shared: SharedDocument,
    messages: Rc<RefCell<Vec<String>>>,
    deadline: Rc<Cell<Instant>>,
    timeout: Duration,
}

impl ScriptRuntime {
    /// Advances the timer clock and runs callbacks that became due.
    pub(crate) fn advance_time(&mut self, milliseconds: u64, compat: &mut CompatReport) {
        self.deadline.set(Instant::now() + self.timeout);
        let messages_before = self.messages.borrow().len();
        let result = self.context.with(|ctx| -> JsResult<()> {
            let advance: Function<'_> = ctx.globals().get("__visionAdvanceTime")?;
            advance.call::<_, ()>((milliseconds,))?;
            ctx.eval::<(), _>("__visionDrainTasks()")?;
            while ctx.execute_pending_job() {}
            Ok(())
        });
        {
            let messages = self.messages.borrow();
            for message in messages.iter().skip(messages_before).take(20) {
                compat.warn(message.clone());
            }
        }
        if let Err(error) = result {
            compat.unsupported(format!("timers: {error}: {}", self.exception_text()));
        }
    }

    fn exception_text(&self) -> String {
        self.context.with(|ctx| exception_text(&ctx))
    }

    /// Dispatches a bubbling DOM event and returns whether it was cancelled.
    pub(crate) fn dispatch(
        &mut self,
        node: NodeId,
        event_type: &str,
        compat: &mut CompatReport,
    ) -> bool {
        self.deadline.set(Instant::now() + self.timeout);
        let messages_before = self.messages.borrow().len();
        let result = self.context.with(|ctx| -> JsResult<bool> {
            let dispatch: Function<'_> = ctx.globals().get("__visionDispatch")?;
            let target = node_object(&ctx, node, &self.shared)?;
            let event: Object<'_> = dispatch.call((target, event_type))?;
            ctx.eval::<(), _>("__visionDrainTasks()")?;
            while ctx.execute_pending_job() {}
            event.get::<_, bool>("defaultPrevented")
        });
        {
            let messages = self.messages.borrow();
            for message in messages.iter().skip(messages_before).take(20) {
                compat.warn(message.clone());
            }
        }
        match result {
            Ok(prevented) => prevented,
            Err(error) => {
                compat.unsupported(format!("event: {event_type}: {error}"));
                false
            }
        }
    }
}

/// Runs every executable script in a document before layout.
///
/// The returned runtime stays alive only when the document has executable
/// scripts; `None` means the renderer can behave exactly like the static path.
pub(crate) fn execute_scripts(
    document: &SharedDocument,
    snapshot: &Snapshot,
    compat: &mut CompatReport,
    limits: ScriptLimits,
) -> Option<ScriptRuntime> {
    let has_script = {
        let document = document.borrow();
        document
            .descendants()
            .any(|node| document.element_name(node) == Some("script"))
    };
    if !has_script {
        return None;
    }
    run_scripts(document, snapshot, compat, limits)
}

fn run_scripts(
    shared: &SharedDocument,
    snapshot: &Snapshot,
    compat: &mut CompatReport,
    limits: ScriptLimits,
) -> Option<ScriptRuntime> {
    let scripts = collect_scripts(shared, snapshot, compat);
    if scripts.is_empty() {
        return None;
    }

    let runtime = match Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            compat.unsupported(format!("script: runtime unavailable: {error}"));
            return None;
        }
    };
    runtime.set_memory_limit(limits.memory_bytes);
    runtime.set_max_stack_size(limits.stack_bytes);
    let deadline = Rc::new(Cell::new(Instant::now() + limits.timeout));
    let interrupt_deadline = Rc::clone(&deadline);
    runtime.set_interrupt_handler(Some(Box::new(move || {
        Instant::now() > interrupt_deadline.get()
    })));
    let context = match Context::full(&runtime) {
        Ok(context) => context,
        Err(error) => {
            compat.unsupported(format!("script: context unavailable: {error}"));
            return None;
        }
    };

    let messages = Rc::new(RefCell::new(Vec::new()));
    context.with(|ctx| {
        if let Err(error) = install_globals(&ctx, shared, &messages) {
            compat.unsupported(format!("script: DOM setup failed: {error}"));
            return;
        }
        for script in scripts {
            if let Err(error) = ctx.eval::<(), _>(script.as_str()) {
                compat.unsupported(format!("script error: {error}: {}", exception_text(&ctx)));
            }
        }
        while ctx.execute_pending_job() {}
        if let Err(error) = ctx.eval::<(), _>("__visionDrainTasks()") {
            compat.unsupported(format!("timers: {error}: {}", exception_text(&ctx)));
        }
        while ctx.execute_pending_job() {}
    });
    for message in messages.borrow().iter().take(20) {
        compat.warn(message.clone());
    }
    Some(ScriptRuntime {
        context,
        _runtime: runtime,
        shared: Rc::clone(shared),
        messages,
        deadline,
        timeout: limits.timeout,
    })
}

fn value_text(value: &Value<'_>) -> String {
    if let Some(value) = value.as_string() {
        return value.to_string().unwrap_or_default();
    }
    if let Some(value) = value.as_int() {
        return value.to_string();
    }
    if let Some(value) = value.as_float() {
        return value.to_string();
    }
    if value.is_null() {
        return "null".to_owned();
    }
    if value.is_undefined() {
        return "undefined".to_owned();
    }
    format!("{value:?}")
}

fn exception_text(ctx: &Ctx<'_>) -> String {
    let caught = ctx.catch();
    if let Some(exception) = caught.as_exception() {
        if let Some(message) = exception.message() {
            return message;
        }
        if let Some(stack) = exception.stack() {
            return stack;
        }
    }
    if let Some(value) = caught.as_string() {
        if let Ok(value) = value.to_string() {
            return value;
        }
    }
    "uncaught JavaScript exception".to_owned()
}

fn collect_scripts(
    shared: &SharedDocument,
    snapshot: &Snapshot,
    compat: &mut CompatReport,
) -> Vec<String> {
    let document = shared.borrow();
    let mut scripts = Vec::new();
    for node in document.descendants() {
        if document.element_name(node) != Some("script") {
            continue;
        }
        let kind = document
            .attr(node, "type")
            .unwrap_or("text/javascript")
            .trim()
            .to_ascii_lowercase();
        if !(kind.is_empty()
            || kind == "module"
            || kind.contains("javascript")
            || kind.contains("ecmascript"))
        {
            continue;
        }
        if kind == "module" {
            compat.unsupported("script: module scripts");
            continue;
        }
        if let Some(src) = document.attr(node, "src") {
            let Some(resource) = snapshot
                .resources
                .iter()
                .find(|resource| resource.url == src)
            else {
                compat.unsupported(format!("script: missing {src}"));
                continue;
            };
            match std::str::from_utf8(&resource.bytes) {
                Ok(code) => scripts.push(code.to_owned()),
                Err(_) => compat.unsupported(format!("script: not UTF-8: {src}")),
            }
        } else {
            scripts.push(document.text_content(node));
        }
    }
    scripts
}

fn install_globals<'js>(
    ctx: &Ctx<'js>,
    shared: &SharedDocument,
    messages: &Rc<RefCell<Vec<String>>>,
) -> JsResult<()> {
    let globals = ctx.globals();
    globals.set("__visionNodes", Object::new(ctx.clone())?)?;
    globals.set("__visionNoop", Function::new(ctx.clone(), || {})?)?;
    let document = document_object(ctx, shared)?;
    globals.set("document", document)?;
    globals.set("window", globals.clone())?;
    globals.set("self", globals.clone())?;
    install_constructors(ctx)?;

    let console = Object::new(ctx.clone())?;
    for name in ["log", "info", "debug"] {
        console.set(name, Function::new(ctx.clone(), || {})?)?;
    }
    for name in ["warn", "error"] {
        let messages = Rc::clone(messages);
        console.set(
            name,
            Function::new(ctx.clone(), move |args: Rest<Value<'_>>| {
                let text = args.0.iter().map(value_text).collect::<Vec<_>>().join(" ");
                messages
                    .borrow_mut()
                    .push(format!("console.{name}: {text}"));
            })?,
        )?;
    }
    globals.set("console", console)?;

    ctx.eval::<(), _>(
        r#"
        globalThis.__visionListeners = [];
        globalThis.__visionAddEventListener = function(node, type, callback) {
            type = String(type);
            for (let i = 0; i < globalThis.__visionListeners.length; i++) {
                const listener = globalThis.__visionListeners[i];
                if (listener.node === node && listener.type === type && listener.callback === callback) {
                    return;
                }
            }
            globalThis.__visionListeners.push({ node: node, type: type, callback: callback });
        };
        globalThis.__visionRemoveEventListener = function(node, type, callback) {
            type = String(type);
            globalThis.__visionListeners = globalThis.__visionListeners.filter(function(listener) {
                return !(listener.node === node && listener.type === type && listener.callback === callback);
            });
        };
        globalThis.__visionDispatch = function(target, type) {
            const event = {
                type: String(type),
                target: target,
                currentTarget: null,
                bubbles: true,
                cancelable: true,
                cancelBubble: false,
                defaultPrevented: false,
                isTrusted: false,
                preventDefault: function() { this.defaultPrevented = true; },
                stopPropagation: function() { this.cancelBubble = true; }
            };
            let current = target;
            while (current) {
                const propertyHandler = current["on" + event.type];
                if (typeof propertyHandler === "function") {
                    propertyHandler.call(current, event);
                }
                const listeners = globalThis.__visionListeners;
                for (let i = 0; i < listeners.length; i++) {
                    const listener = listeners[i];
                    if (listener.node !== current.__node || listener.type !== event.type) {
                        continue;
                    }
                    event.currentTarget = current;
                    if (typeof listener.callback === "function") {
                        listener.callback.call(current, event);
                    } else if (listener.callback && typeof listener.callback.handleEvent === "function") {
                        listener.callback.handleEvent(event);
                    }
                    if (event.cancelBubble) {
                        break;
                    }
                }
                if (event.cancelBubble) {
                    break;
                }
                current = current.parentNode;
            }
            event.currentTarget = null;
            return event;
        };
        "#,
    )?;

    let root = shared.borrow().root();
    let add_listener = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, kind: String, callback: Function<'js>| -> JsResult<()> {
            let add: Function<'_> = ctx.globals().get("__visionAddEventListener")?;
            add.call::<_, ()>((root.0, kind, callback))
        },
    )?;
    let remove_listener = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, kind: String, callback: Function<'js>| -> JsResult<()> {
            let remove: Function<'_> = ctx.globals().get("__visionRemoveEventListener")?;
            remove.call::<_, ()>((root.0, kind, callback))
        },
    )?;
    globals.set("addEventListener", add_listener)?;
    globals.set("removeEventListener", remove_listener)?;

    ctx.eval::<(), _>(
        r#"
        globalThis.__visionStart = Date.now();
        globalThis.__visionNow = 0;
        globalThis.__visionOffset = 0;
        globalThis.__visionTaskId = 1;
        globalThis.__visionTasks = [];
        globalThis.__visionMicrotasks = [];
        globalThis.__visionTick = function() {
            const elapsed = Math.max(0, Date.now() - globalThis.__visionStart) + globalThis.__visionOffset;
            if (elapsed > globalThis.__visionNow) {
                globalThis.__visionNow = elapsed;
            }
            return globalThis.__visionNow;
        };
        globalThis.__visionAdvanceTime = function(milliseconds) {
            const wait = Math.max(0, Number(milliseconds) || 0);
            globalThis.__visionOffset += wait;
            return globalThis.__visionTick();
        };
        globalThis.performance = {
            now: function() { return globalThis.__visionTick(); },
            timeOrigin: globalThis.__visionStart
        };
        globalThis.setTimeout = function(callback, delay) {
            const id = globalThis.__visionTaskId++;
            const wait = Math.max(0, Number(delay) || 0);
            globalThis.__visionTasks.push({
                id: id,
                callback: callback,
                args: Array.prototype.slice.call(arguments, 2),
                due: globalThis.__visionTick() + wait,
                interval: 0
            });
            return id;
        };
        globalThis.setInterval = function(callback, delay) {
            const id = globalThis.__visionTaskId++;
            const wait = Math.max(1, Number(delay) || 1);
            globalThis.__visionTasks.push({
                id: id,
                callback: callback,
                args: Array.prototype.slice.call(arguments, 2),
                due: globalThis.__visionTick() + wait,
                interval: wait
            });
            return id;
        };
        globalThis.clearTimeout = function(id) {
            globalThis.__visionTasks = globalThis.__visionTasks.filter(function(task) {
                return task.id !== id;
            });
        };
        globalThis.clearInterval = globalThis.clearTimeout;
        globalThis.queueMicrotask = function(callback) {
            globalThis.__visionMicrotasks.push(callback);
        };
        globalThis.requestAnimationFrame = function(callback) {
            const id = globalThis.__visionTaskId++;
            const frame = globalThis.__visionTick() + 16;
            globalThis.__visionTasks.push({
                id: id,
                callback: callback,
                args: [frame],
                due: frame,
                interval: 0
            });
            return id;
        };
        globalThis.cancelAnimationFrame = globalThis.clearTimeout;
        globalThis.__visionDrainTasks = function() {
            globalThis.__visionTick();
            let runs = 0;
            while (runs++ < 10000) {
                if (globalThis.__visionMicrotasks.length) {
                    const callback = globalThis.__visionMicrotasks.shift();
                    callback();
                    continue;
                }
                let nextIndex = -1;
                let nextDue = Infinity;
                for (let i = 0; i < globalThis.__visionTasks.length; i++) {
                    const task = globalThis.__visionTasks[i];
                    if (task.due <= globalThis.__visionNow && task.due < nextDue) {
                        nextIndex = i;
                        nextDue = task.due;
                    }
                }
                if (nextIndex < 0) {
                    break;
                }
                const task = globalThis.__visionTasks[nextIndex];
                if (task.interval > 0) {
                    task.due = task.due + task.interval;
                } else {
                    globalThis.__visionTasks.splice(nextIndex, 1);
                }
                if (typeof task.callback === "function") {
                    task.callback.apply(globalThis, task.args);
                }
            }
            return globalThis.__visionNow;
        };
        "#,
    )?;
    Ok(())
}

fn document_object<'js>(ctx: &Ctx<'js>, shared: &SharedDocument) -> JsResult<Object<'js>> {
    let object = Object::new(ctx.clone())?;

    let document = Rc::clone(shared);
    object.set(
        "getElementById",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, id: String| -> JsResult<Value<'js>> {
                let node = {
                    let document = document.borrow();
                    document
                        .descendants()
                        .find(|node| document.attr(*node, "id") == Some(id.as_str()))
                };
                match node {
                    Some(node) => Ok(node_object(&ctx, node, &document)?.into_value()),
                    None => Ok(Value::new_null(ctx)),
                }
            },
        )?,
    )?;

    let document = Rc::clone(shared);
    object.set(
        "querySelector",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, selector: String| -> JsResult<Value<'js>> {
                let node = {
                    let document = document.borrow();
                    let selectors = parse_selector_list(&selector);
                    if selectors.is_empty() {
                        None
                    } else {
                        document.descendants().find(|node| {
                            document.element(*node).is_some()
                                && selectors
                                    .iter()
                                    .any(|selector| selector.matches(&document, *node))
                        })
                    }
                };
                match node {
                    Some(node) => Ok(node_object(&ctx, node, &document)?.into_value()),
                    None => Ok(Value::new_null(ctx)),
                }
            },
        )?,
    )?;

    let document = Rc::clone(shared);
    object.set(
        "querySelectorAll",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, selector: String| -> JsResult<Array<'js>> {
                let nodes = {
                    let document = document.borrow();
                    let selectors = parse_selector_list(&selector);
                    if selectors.is_empty() {
                        Vec::new()
                    } else {
                        document
                            .descendants()
                            .filter(|node| {
                                document.element(*node).is_some()
                                    && selectors
                                        .iter()
                                        .any(|selector| selector.matches(&document, *node))
                            })
                            .collect::<Vec<_>>()
                    }
                };
                let array = Array::new(ctx.clone())?;
                for (index, node) in nodes.into_iter().enumerate() {
                    array.set(index, node_object(&ctx, node, &document)?)?;
                }
                Ok(array)
            },
        )?,
    )?;

    let document = Rc::clone(shared);
    object.set(
        "createElement",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, name: String| -> JsResult<Object<'js>> {
                let node = document
                    .borrow_mut()
                    .create_element(name.to_ascii_lowercase(), Vec::new());
                node_object(&ctx, node, &document)
            },
        )?,
    )?;

    let document = Rc::clone(shared);
    object.set(
        "createTextNode",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, text: String| -> JsResult<Object<'js>> {
                let node = document.borrow_mut().create_text(text);
                node_object(&ctx, node, &document)
            },
        )?,
    )?;

    for (name, element) in [
        ("body", "body"),
        ("head", "head"),
        ("documentElement", "html"),
    ] {
        let node = {
            let document = shared.borrow();
            document.find_first_element(element)
        };
        let value = match node {
            Some(node) => node_object(ctx, node, shared)?.into_value(),
            None => Value::new_null(ctx.clone()),
        };
        object.set(name, value)?;
    }
    let root = shared.borrow().root();
    object.set(
        "addEventListener",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, kind: String, callback: Function<'js>| -> JsResult<()> {
                let add: Function<'_> = ctx.globals().get("__visionAddEventListener")?;
                add.call::<_, ()>((root.0, kind, callback))
            },
        )?,
    )?;
    object.set(
        "removeEventListener",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, kind: String, callback: Function<'js>| -> JsResult<()> {
                let remove: Function<'_> = ctx.globals().get("__visionRemoveEventListener")?;
                remove.call::<_, ()>((root.0, kind, callback))
            },
        )?,
    )?;
    object.set("nodeType", 9)?;
    object.set("nodeName", "#document")?;
    object.set("readyState", "complete")?;
    object.set("defaultView", ctx.globals())?;
    let _ = set_constructors(ctx, &object);
    Ok(object)
}

fn owner_document_object<'js>(ctx: &Ctx<'js>, shared: &SharedDocument) -> JsResult<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set("nodeType", 9)?;
    object.set("nodeName", "#document")?;
    let document = Rc::clone(shared);
    object.set(
        "createElement",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, name: String| -> JsResult<Object<'js>> {
                let node = document
                    .borrow_mut()
                    .create_element(name.to_ascii_lowercase(), Vec::new());
                node_object(&ctx, node, &document)
            },
        )?,
    )?;
    let document = Rc::clone(shared);
    object.set(
        "createElementNS",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, _namespace: String, name: String| -> JsResult<Object<'js>> {
                let node = document
                    .borrow_mut()
                    .create_element(name.to_ascii_lowercase(), Vec::new());
                node_object(&ctx, node, &document)
            },
        )?,
    )?;
    let document = Rc::clone(shared);
    object.set(
        "createTextNode",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, text: String| -> JsResult<Object<'js>> {
                let node = document.borrow_mut().create_text(text);
                node_object(&ctx, node, &document)
            },
        )?,
    )?;
    object.set("addEventListener", Function::new(ctx.clone(), || {})?)?;
    object.set("defaultView", ctx.globals())?;
    let _ = set_constructors(ctx, &object);
    Ok(object)
}

fn install_constructors(ctx: &Ctx<'_>) -> JsResult<()> {
    ctx.eval::<(), _>(
        "for (const name of ['Node','Element','HTMLElement','HTMLIFrameElement',         'HTMLInputElement','HTMLTextAreaElement','HTMLSelectElement','HTMLButtonElement',         'HTMLAnchorElement','HTMLImageElement','HTMLFormElement','HTMLDivElement',         'HTMLSpanElement','HTMLScriptElement','HTMLTemplateElement','HTMLSlotElement',         'HTMLTableElement','HTMLDocument','Document','Text','Comment','AbortSignal',         'SVGElement','MathMLElement']) { const C = function() {}; C.prototype = {};          globalThis[name] = C; }",
    )
}

fn set_constructors<'js>(ctx: &Ctx<'js>, object: &Object<'js>) -> JsResult<()> {
    let globals = ctx.globals();
    for name in [
        "Node",
        "Element",
        "HTMLElement",
        "HTMLIFrameElement",
        "HTMLInputElement",
        "HTMLTextAreaElement",
        "HTMLSelectElement",
        "HTMLButtonElement",
        "HTMLAnchorElement",
        "HTMLImageElement",
        "HTMLFormElement",
        "HTMLDivElement",
        "HTMLSpanElement",
        "HTMLScriptElement",
        "HTMLTemplateElement",
        "HTMLSlotElement",
        "HTMLTableElement",
        "HTMLDocument",
        "Document",
        "Text",
        "Comment",
        "AbortSignal",
        "SVGElement",
        "MathMLElement",
    ] {
        if let Ok(value) = globals.get::<_, Value<'_>>(name) {
            object.set(name, value)?;
        }
    }
    Ok(())
}

fn node_object<'js>(
    ctx: &Ctx<'js>,
    node: NodeId,
    shared: &SharedDocument,
) -> JsResult<Object<'js>> {
    let cache: Object<'js> = ctx.globals().get("__visionNodes")?;
    let cached: Value<'js> = cache.get(node.0)?;
    if let Some(object) = cached.into_object() {
        return Ok(object);
    }
    let object = Object::new(ctx.clone())?;
    object.set("__node", node.0)?;

    let is_element = shared.borrow().element(node).is_some();
    object.set("nodeType", if is_element { 1 } else { 3 })?;
    let node_name = if is_element {
        shared
            .borrow()
            .element_name(node)
            .unwrap_or_default()
            .to_ascii_uppercase()
    } else {
        "#text".to_owned()
    };
    object.set("nodeName", node_name.clone())?;
    if is_element {
        object.set("tagName", node_name)?;
        object.set(
            "localName",
            shared.borrow().element_name(node).unwrap_or_default(),
        )?;
    }

    let document = Rc::clone(shared);
    object.prop(
        "textContent",
        Accessor::from(move || document.borrow().text_content(node)).set({
            let document = Rc::clone(shared);
            move |value: String| {
                document.borrow_mut().set_text_content(node, &value);
            }
        }),
    )?;

    let document = Rc::clone(shared);
    object.prop(
        "innerHTML",
        Accessor::from(move || {
            let document = document.borrow();
            serialize_children(&document, node)
        })
        .set({
            let document = Rc::clone(shared);
            move |value: String| {
                set_inner_html(&mut document.borrow_mut(), node, &value);
            }
        }),
    )?;

    let document = Rc::clone(shared);
    object.prop(
        "outerHTML",
        Accessor::from(move || {
            let document = document.borrow();
            serialize_html(&document, node)
        }),
    )?;

    let document = Rc::clone(shared);
    object.prop(
        "id",
        Accessor::from(move || document.borrow().attr(node, "id").unwrap_or("").to_owned()).set({
            let document = Rc::clone(shared);
            move |value: String| {
                document.borrow_mut().set_attr(node, "id", value);
            }
        }),
    )?;

    let document = Rc::clone(shared);
    object.prop(
        "type",
        Accessor::from(move || {
            let document = document.borrow();
            document
                .attr(node, "type")
                .map(str::to_owned)
                .or_else(|| {
                    (document.element_name(node) == Some("input")).then(|| "text".to_owned())
                })
                .unwrap_or_default()
        })
        .set({
            let document = Rc::clone(shared);
            move |value: String| {
                document.borrow_mut().set_attr(node, "type", value);
            }
        }),
    )?;

    let document = Rc::clone(shared);
    object.prop(
        "value",
        Accessor::from(move || {
            document
                .borrow()
                .attr(node, "value")
                .unwrap_or("")
                .to_owned()
        })
        .set({
            let document = Rc::clone(shared);
            move |value: String| {
                document.borrow_mut().set_attr(node, "value", value);
            }
        }),
    )?;

    let document = Rc::clone(shared);
    object.prop(
        "checked",
        Accessor::from(move || document.borrow().attr(node, "checked").is_some()).set({
            let document = Rc::clone(shared);
            move |value: bool| {
                if value {
                    document.borrow_mut().set_attr(node, "checked", "");
                } else {
                    document.borrow_mut().remove_attr(node, "checked");
                }
            }
        }),
    )?;

    let document = Rc::clone(shared);
    object.prop(
        "nodeValue",
        Accessor::from(move |ctx: Ctx<'js>| -> JsResult<Value<'js>> {
            let text = {
                let document = document.borrow();
                if document.element(node).is_some() {
                    None
                } else {
                    Some(document.text_content(node))
                }
            };
            match text {
                Some(text) => Ok(rquickjs::String::from_str(ctx, &text)?.into_value()),
                None => Ok(Value::new_null(ctx)),
            }
        })
        .set({
            let document = Rc::clone(shared);
            move |value: String| {
                document.borrow_mut().set_text_content(node, &value);
            }
        }),
    )?;

    let document = Rc::clone(shared);
    object.prop(
        "className",
        Accessor::from(move || {
            document
                .borrow()
                .attr(node, "class")
                .unwrap_or("")
                .to_owned()
        })
        .set({
            let document = Rc::clone(shared);
            move |value: String| {
                document.borrow_mut().set_attr(node, "class", value);
            }
        }),
    )?;

    let document = Rc::clone(shared);
    object.set(
        "setAttribute",
        Function::new(ctx.clone(), move |name: String, value: String| {
            document
                .borrow_mut()
                .set_attr(node, &name.to_ascii_lowercase(), value);
        })?,
    )?;
    let document = Rc::clone(shared);
    object.set(
        "getAttribute",
        Function::new(ctx.clone(), move |name: String| {
            document
                .borrow()
                .attr(node, &name.to_ascii_lowercase())
                .map(str::to_owned)
        })?,
    )?;
    let document = Rc::clone(shared);
    object.set(
        "hasAttribute",
        Function::new(ctx.clone(), move |name: String| {
            document
                .borrow()
                .attr(node, &name.to_ascii_lowercase())
                .is_some()
        })?,
    )?;
    let document = Rc::clone(shared);
    object.set(
        "removeAttribute",
        Function::new(ctx.clone(), move |name: String| {
            document
                .borrow_mut()
                .remove_attr(node, &name.to_ascii_lowercase());
        })?,
    )?;

    let document = Rc::clone(shared);
    object.set(
        "appendChild",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, child: Object<'js>| -> JsResult<Object<'js>> {
                let child_node = NodeId(child.get::<_, u32>("__node")?);
                document.borrow_mut().append(node, child_node);
                node_object(&ctx, child_node, &document)
            },
        )?,
    )?;
    let document = Rc::clone(shared);
    object.set(
        "insertBefore",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  child: Object<'js>,
                  reference: Option<Object<'js>>|
                  -> JsResult<Object<'js>> {
                let child_node = NodeId(child.get::<_, u32>("__node")?);
                if let Some(reference) = reference {
                    let reference_node = NodeId(reference.get::<_, u32>("__node")?);
                    document
                        .borrow_mut()
                        .insert_before(node, child_node, reference_node);
                } else {
                    document.borrow_mut().append(node, child_node);
                }
                node_object(&ctx, child_node, &document)
            },
        )?,
    )?;

    let document = Rc::clone(shared);
    object.set(
        "removeChild",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, child: Object<'js>| -> JsResult<Object<'js>> {
                let child_node = NodeId(child.get::<_, u32>("__node")?);
                document.borrow_mut().remove_child(node, child_node);
                node_object(&ctx, child_node, &document)
            },
        )?,
    )?;
    let document = Rc::clone(shared);
    object.set(
        "remove",
        Function::new(ctx.clone(), move || {
            document.borrow_mut().detach(node);
        })?,
    )?;
    object.set(
        "addEventListener",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, kind: String, callback: Function<'js>| -> JsResult<()> {
                let add: Function<'_> = ctx.globals().get("__visionAddEventListener")?;
                add.call::<_, ()>((node.0, kind, callback))
            },
        )?,
    )?;
    object.set(
        "removeEventListener",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, kind: String, callback: Function<'js>| -> JsResult<()> {
                let remove: Function<'_> = ctx.globals().get("__visionRemoveEventListener")?;
                remove.call::<_, ()>((node.0, kind, callback))
            },
        )?,
    )?;

    object.set("ownerDocument", owner_document_object(ctx, shared)?)?;

    let document = Rc::clone(shared);
    object.prop(
        "parentNode",
        Accessor::from(move |ctx: Ctx<'js>| -> JsResult<Value<'js>> {
            let parent = document.borrow().parent(node);
            match parent {
                Some(parent) => Ok(node_object(&ctx, parent, &document)?.into_value()),
                None => Ok(Value::new_null(ctx)),
            }
        }),
    )?;

    let document = Rc::clone(shared);
    object.prop(
        "attributes",
        Accessor::from(move |ctx: Ctx<'js>| -> JsResult<Array<'js>> {
            let attributes = {
                let document = document.borrow();
                document
                    .element(node)
                    .map(|element| {
                        element
                            .attrs
                            .iter()
                            .map(|attribute| (attribute.name.clone(), attribute.value.clone()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            };
            let array = Array::new(ctx.clone())?;
            for (index, (name, value)) in attributes.into_iter().enumerate() {
                let attribute = Object::new(ctx.clone())?;
                attribute.set("name", name.clone())?;
                attribute.set("localName", name)?;
                attribute.set("value", value)?;
                attribute.set("specified", true)?;
                attribute.set("namespaceURI", Value::new_null(ctx.clone()))?;
                attribute.set("prefix", Value::new_null(ctx.clone()))?;
                array.set(index, attribute)?;
            }
            Ok(array)
        }),
    )?;

    for (name, last) in [("firstChild", false), ("lastChild", true)] {
        let document = Rc::clone(shared);
        object.prop(
            name,
            Accessor::from(move |ctx: Ctx<'js>| -> JsResult<Value<'js>> {
                let child = {
                    let document = document.borrow();
                    if last {
                        document.last_child(node)
                    } else {
                        document.first_child(node)
                    }
                };
                match child {
                    Some(child) => Ok(node_object(&ctx, child, &document)?.into_value()),
                    None => Ok(Value::new_null(ctx)),
                }
            }),
        )?;
    }

    let document = Rc::clone(shared);
    object.prop(
        "childNodes",
        Accessor::from(move |ctx: Ctx<'js>| -> JsResult<Array<'js>> {
            let children = {
                let document = document.borrow();
                document.children(node).collect::<Vec<_>>()
            };
            let array = Array::new(ctx.clone())?;
            for (index, child) in children.into_iter().enumerate() {
                array.set(index, node_object(&ctx, child, &document)?)?;
            }
            Ok(array)
        }),
    )?;

    for (name, previous) in [("nextSibling", false), ("previousSibling", true)] {
        let document = Rc::clone(shared);
        object.prop(
            name,
            Accessor::from(move |ctx: Ctx<'js>| -> JsResult<Value<'js>> {
                let sibling = {
                    let document = document.borrow();
                    if previous {
                        document.node(node).prev_sibling
                    } else {
                        document.node(node).next_sibling
                    }
                };
                match sibling {
                    Some(sibling) => Ok(node_object(&ctx, sibling, &document)?.into_value()),
                    None => Ok(Value::new_null(ctx)),
                }
            }),
        )?;
    }

    let document = Rc::clone(shared);
    object.prop(
        "children",
        Accessor::from(move |ctx: Ctx<'js>| -> JsResult<Array<'js>> {
            let nodes = {
                let document = document.borrow();
                document
                    .children(node)
                    .filter(|child| document.element(*child).is_some())
                    .collect::<Vec<_>>()
            };
            let array = Array::new(ctx.clone())?;
            for (index, child) in nodes.into_iter().enumerate() {
                array.set(index, node_object(&ctx, child, &document)?)?;
            }
            Ok(array)
        }),
    )?;

    cache.set(node.0, object.clone())?;

    // React probes `document.createElement("div").oninput` to decide whether
    // native input events are available. Keep one shared no-op handler rather
    // than allocating event-property closures for every node.
    let noop: Function<'_> = ctx.globals().get("__visionNoop")?;
    object.set("oninput", noop)?;

    let document = Rc::clone(shared);
    object.set("classList", class_list_object(ctx, node, document)?)?;
    let document = Rc::clone(shared);
    object.set("style", style_object(ctx, node, document)?)?;
    Ok(object)
}

fn class_list_object<'js>(
    ctx: &Ctx<'js>,
    node: NodeId,
    document: SharedDocument,
) -> JsResult<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    let document_add = Rc::clone(&document);
    object.set(
        "add",
        Function::new(ctx.clone(), move |name: String| {
            let mut document = document_add.borrow_mut();
            let mut classes = class_names(&document, node);
            if !classes.iter().any(|class| class == &name) {
                classes.push(name);
                document.set_attr(node, "class", classes.join(" "));
            }
        })?,
    )?;
    let document_remove = Rc::clone(&document);
    object.set(
        "remove",
        Function::new(ctx.clone(), move |name: String| {
            let mut document = document_remove.borrow_mut();
            let mut classes = class_names(&document, node);
            classes.retain(|class| class != &name);
            document.set_attr(node, "class", classes.join(" "));
        })?,
    )?;
    let document_contains = Rc::clone(&document);
    object.set(
        "contains",
        Function::new(ctx.clone(), move |name: String| {
            class_names(&document_contains.borrow(), node)
                .iter()
                .any(|class| class == &name)
        })?,
    )?;
    object.set(
        "toggle",
        Function::new(ctx.clone(), move |name: String| {
            let mut document = document.borrow_mut();
            let mut classes = class_names(&document, node);
            let present = classes.iter().any(|class| class == &name);
            if present {
                classes.retain(|class| class != &name);
            } else {
                classes.push(name);
            }
            document.set_attr(node, "class", classes.join(" "));
            !present
        })?,
    )?;
    Ok(object)
}

fn style_object<'js>(
    ctx: &Ctx<'js>,
    node: NodeId,
    document: SharedDocument,
) -> JsResult<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    let document_set = Rc::clone(&document);
    object.set(
        "setProperty",
        Function::new(ctx.clone(), move |name: String, value: String| {
            set_style_property(&mut document_set.borrow_mut(), node, &name, &value);
        })?,
    )?;
    let document_get = Rc::clone(&document);
    object.set(
        "getPropertyValue",
        Function::new(ctx.clone(), move |name: String| {
            get_style_property(&document_get.borrow(), node, &name)
        })?,
    )?;
    let document_get = Rc::clone(&document);
    object.prop(
        "cssText",
        Accessor::from(move || {
            document_get
                .borrow()
                .attr(node, "style")
                .unwrap_or("")
                .to_owned()
        })
        .set({
            let document = Rc::clone(&document);
            move |value: String| {
                document.borrow_mut().set_attr(node, "style", value);
            }
        }),
    )?;
    Ok(object)
}

fn class_names(document: &Document, node: NodeId) -> Vec<String> {
    document
        .attr(node, "class")
        .unwrap_or("")
        .split_ascii_whitespace()
        .map(str::to_owned)
        .collect()
}

fn set_style_property(document: &mut Document, node: NodeId, name: &str, value: &str) {
    let current = document.attr(node, "style").unwrap_or("").to_owned();
    let mut declarations = current
        .split(';')
        .filter_map(|declaration| {
            let (key, old_value) = declaration.split_once(':')?;
            Some((key.trim().to_owned(), old_value.trim().to_owned()))
        })
        .collect::<Vec<_>>();
    let name = name.trim().to_ascii_lowercase();
    let mut found = false;
    for (key, old_value) in &mut declarations {
        if key.eq_ignore_ascii_case(&name) {
            *old_value = value.to_owned();
            found = true;
            break;
        }
    }
    if !found {
        declarations.push((name, value.to_owned()));
    }
    let style = declarations
        .into_iter()
        .map(|(key, value)| format!("{key}: {value}"))
        .collect::<Vec<_>>()
        .join("; ");
    document.set_attr(node, "style", style);
}

fn get_style_property(document: &Document, node: NodeId, name: &str) -> String {
    let current = document.attr(node, "style").unwrap_or("");
    current
        .split(';')
        .filter_map(|declaration| {
            let (key, value) = declaration.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case(name.trim())
                .then(|| value.trim().to_owned())
        })
        .next()
        .unwrap_or_default()
}

fn set_inner_html(document: &mut Document, node: NodeId, html: &str) {
    document.clear_children(node);
    let Ok(fragment) = Document::parse(&format!("<body>{html}</body>")) else {
        return;
    };
    let Some(body) = fragment.find_first_element("body") else {
        return;
    };
    for child in fragment.children(body).collect::<Vec<_>>() {
        fragment.copy_subtree_into(child, document, node);
    }
}

fn serialize_html(document: &Document, node: NodeId) -> String {
    let mut output = String::new();
    serialize_node(document, node, &mut output);
    output
}

fn serialize_children(document: &Document, node: NodeId) -> String {
    let mut output = String::new();
    for child in document.children(node) {
        serialize_node(document, child, &mut output);
    }
    output
}

enum SerializeTask {
    Node(NodeId),
    CloseElement(String),
}

fn serialize_node(document: &Document, node: NodeId, output: &mut String) {
    let mut stack = vec![SerializeTask::Node(node)];
    while let Some(task) = stack.pop() {
        match task {
            SerializeTask::Node(node) => match &document.node(node).kind {
                NodeKind::Text(text) => {
                    output.push_str(&text.replace('&', "&amp;").replace('<', "&lt;"));
                }
                NodeKind::Element(element) => {
                    output.push('<');
                    output.push_str(&element.name);
                    for attribute in &element.attrs {
                        output.push(' ');
                        output.push_str(&attribute.name);
                        output.push_str("=\"");
                        output.push_str(
                            &attribute.value.replace('&', "&amp;").replace('"', "&quot;"),
                        );
                        output.push('"');
                    }
                    output.push('>');
                    stack.push(SerializeTask::CloseElement(element.name.clone()));
                    let children: Vec<_> = document.children(node).collect();
                    for child in children.into_iter().rev() {
                        stack.push(SerializeTask::Node(child));
                    }
                }
                NodeKind::Document => {
                    let children: Vec<_> = document.children(node).collect();
                    for child in children.into_iter().rev() {
                        stack.push(SerializeTask::Node(child));
                    }
                }
            },
            SerializeTask::CloseElement(name) => {
                output.push_str("</");
                output.push_str(&name);
                output.push('>');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deep_document(depth: usize) -> Document {
        let mut document = Document::new();
        let mut parent = document.root();
        for _ in 0..depth {
            let node = document.create_element("div", Vec::new());
            document.append(parent, node);
            parent = node;
        }
        document
    }

    #[test]
    fn serializes_deep_dom_without_recursion() {
        let document = deep_document(20_000);
        let html = serialize_html(&document, document.root());
        assert_eq!(html.matches("<div>").count(), 20_000);
        assert_eq!(html.matches("</div>").count(), 20_000);
    }

    #[test]
    fn copies_deep_subtrees_without_recursion() {
        let document = deep_document(20_000);
        let Some(first) = document.children(document.root()).next() else {
            panic!("deep document has a root element");
        };
        let mut target = Document::new();
        let target_root = target.root();
        let copied = document.copy_subtree_into(first, &mut target, target_root);
        assert_eq!(target.len(), 20_001);
        assert_eq!(target.element_name(copied), Some("div"));
    }
}
