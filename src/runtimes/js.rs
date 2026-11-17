//! JavaScript via Boa. `icp.call` / `icp.candid` return pending Promises; the
//! host drains the job queue, dispatches every pending request concurrently
//! (so `Promise.all` becomes a true batch), settles the promises and repeats.

use crate::bridge::{HostBridge, MAX_LOG_BYTES, budget_error, check_budget, log_line, normalize_numbers, now_ms, truncate};
use boa_engine::builtins::promise::{PromiseState, ResolvingFunctions};
use boa_engine::context::time::FixedClock;
use boa_engine::context::{ContextBuilder, HostHooks};
use boa_engine::job::{GenericJob, Job, JobExecutor, NativeAsyncJob, PromiseJob};
use boa_engine::module::IdleModuleLoader;
use boa_engine::object::ObjectInitializer;
use boa_engine::object::builtins::{JsFunction, JsPromise};
use boa_engine::property::Attribute;
use boa_engine::realm::Realm;
use boa_engine::{Context, JsError, JsNativeError, JsResult, JsString, JsValue, NativeFunction, Source, js_string};
use boa_gc::{Finalize, Gc, GcRefCell, Trace};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

pub const RECURSION_LIMIT: usize = 256;
pub const STACK_SIZE_LIMIT: usize = 10 * 1024;
/// Host round-trips (each one a batch of concurrent requests).
pub const MAX_ROUNDS: usize = 64;
/// Concurrent `icp.*` requests per round.
pub use super::MAX_PENDING;
/// Values crossing into Rust (results, call args, console.log) are snapshotted with
/// these bounds; Boa's own `to_json` / `JSON.stringify` recurse without a limit and
/// would overflow the wasm stack.
pub const MAX_VALUE_DEPTH: usize = crate::candid_bridge::MAX_DEPTH;
pub const MAX_VALUE_NODES: usize = crate::bridge::MAX_VALUE_NODES;
pub const MAX_VALUE_BYTES: usize = crate::bridge::MAX_VALUE_BYTES;
/// Boa VM steps (bytecode instructions) per run, across every function, callback and
/// promise job. Measured on a local replica at about 330 instructions per step, so 30M
/// steps is about 10B, leaving room under 40B for steps several times heavier. There is
/// no per-loop limit: this cap and the instruction deadline (checked by the VM interrupt,
/// see `Meter`) bound all work, so one loop may run as long as several.
pub const MAX_VM_STEPS: usize = 30_000_000;
/// Heap one run may hold, across awaits included. Eight runs at this size stay well below
/// the canister-wide `HEAP_CEILING` (1 GiB), past which any allocation traps.
pub const JS_MAX_MEMORY: usize = 96 << 20;
/// Native stack that property access and calls may use below the host loop before they
/// throw a RangeError (vendor/boa_engine PATCHES.md). The canister's stack is 8 MiB; tests
/// run on 2 MiB threads.
const NATIVE_STACK: usize = if cfg!(target_arch = "wasm32") { 5 << 20 } else { 1 << 20 };
/// Deepest nesting a script's source may have (brackets plus chained operators, see
/// `check_nesting`). Boa's parser recurses natively for each level, about 18 KB of wasm
/// stack per parenthesis.
pub const MAX_NESTING: usize = 128;
/// Longest error text returned for a script.
const MAX_ERROR_BYTES: usize = 8192;
/// Longest string `padStart`/`padEnd`/`repeat` may build (Boa allocates it up front).
pub const MAX_STRING_LEN: usize = 1 << 24;
/// Promise jobs (microtasks) per script. A chain like `f = () => Promise.resolve().then(f)`
/// has no loop and no call depth, so the loop and recursion limits never see it.
pub const MAX_PROMISE_JOBS: u64 = 100_000;

/// Installed before user code, so it only holds pristine builtins. Reads every value
/// exactly once (getters cannot hand `to_json` a different, deeper value later), and
/// returns a plain copy: own data properties only, bounded in depth, element count
/// (every value counts, primitives included) and total text size, `toJSON` honoured,
/// functions/symbols dropped like `JSON.stringify` does. Also wraps `JSON.stringify`
/// with the same bounds, replacer included. Evaluates to `[snapshot, show]`; `show`
/// formats any value for `console.log` and error text.
///
/// With `host` set (values leaving the script), the rules all runtimes share apply where
/// JSON has none: NaN and infinities become "NaN"/"inf"/"-inf", errors their text, typed
/// arrays arrays of their elements, ArrayBuffers and DataViews byte arrays, and lone
/// surrogates U+FFFD (as Python's do).
const SNAPSHOT_JS: &str = r#"(() => {
  const keys = Object.keys, isArray = Array.isArray, def = Object.defineProperty, str = JSON.stringify, apply = Reflect.apply;
  const create = Object.create, Str = String, objToString = Object.prototype.toString, sliceS = String.prototype.slice;
  const MAXD = __MAXD__, MAXN = __MAXN__, MAXB = __MAXB__;
  const put = (o, k, v) => def(o, k, { value: v, writable: true, enumerable: true, configurable: true });
  const cut = s => s.length > 4096 ? apply(sliceS, s, [0, 4096]) + "…" : s;
  const errText = v => {
    const n = v.name, m = v.message, name = typeof n === "string" ? cut(n) : "Error";
    return typeof m === "string" && m ? name + ": " + cut(m) : name;
  };
  const gd = Object.getOwnPropertyDescriptor, from = Array.from, U8 = Uint8Array, isView = ArrayBuffer.isView, isFin = Number.isFinite;
  const taLen = gd(Object.getPrototypeOf(U8.prototype), "length").get, abLen = gd(ArrayBuffer.prototype, "byteLength").get;
  const isWF = String.prototype.isWellFormed, toWF = String.prototype.toWellFormed;
  const wellFormed = s => apply(isWF, s, []) ? s : apply(toWF, s, []);
  const len = (g, x) => { try { return apply(g, x, []); } catch (_) { return -1; } };
  // Typed arrays as arrays of their elements; ArrayBuffers and DataViews as byte arrays.
  // `tag`: the value's Object.prototype.toString text (no exception for ordinary objects).
  const plain = (x, tag) => {
    let n = -1;
    if (isView(x)) {
      n = len(taLen, x);
      if (n < 0) { x = new U8(x.buffer, x.byteOffset, x.byteLength); n = len(taLen, x); }
    } else if (tag === "[object ArrayBuffer]" && len(abLen, x) >= 0) { x = new U8(x); n = len(taLen, x); }
    if (n < 0) return undefined;
    if (n > MAXN) throw new RangeError("value has more than " + MAXN + " elements");
    return apply(from, null, [x]);
  };
  // Pristine valueOf methods: each throws unless `this` really is that kind of box.
  const UNBOX = [Number.prototype.valueOf, String.prototype.valueOf, Boolean.prototype.valueOf, BigInt.prototype.valueOf];
  const unbox = x => {
    for (let j = 0; j < UNBOX.length; j++) {
      try { return apply(UNBOX[j], x, []); } catch (_) {}
    }
    return x;
  };
  // `replacer`: optional JSON.stringify replacer function, applied here (inside the bounds)
  // in the order the spec gives: toJSON, then the replacer, then the type rules.
  // `list`: an array replacer's property list; objects then serialise only those keys.
  const snapshot = (root, replacer, list, host) => {
    let nodes = 0, bytes = 0;
    const walk = (x, d, holder, key) => {
      let t = typeof x;
      if ((t === "object" && x !== null) || t === "bigint") {
        const tj = x.toJSON;
        if (typeof tj === "function") { x = apply(tj, x, [key]); t = typeof x; }
      }
      if (replacer) { x = apply(replacer, holder, [key, x]); t = typeof x; }
      // Boxed primitives (new Number(1), Object("s"), ...) serialise as their value.
      if (t === "object" && x !== null) { x = unbox(x); t = typeof x; }
      if (host) {
        if (t === "number" && !isFin(x)) { x = x !== x ? "NaN" : x > 0 ? "inf" : "-inf"; t = "string"; }
        if (t === "object" && x !== null) {
          const tag = apply(objToString, x, []);
          if (tag === "[object Error]") { x = errText(x); t = "string"; }
          else { const b = plain(x, tag); if (b !== undefined) x = b; }
        }
        if (t === "string") x = wellFormed(x);
      }
      if (++nodes > MAXN) throw new RangeError("value has more than " + MAXN + " elements");
      if (t === "bigint") throw new TypeError("BigInt values cannot cross into the host; convert to string");
      if (t === "function" || t === "symbol") return undefined;
      if (t === "string") {
        bytes += x.length;
        if (bytes > MAXB) throw new RangeError("value holds more than " + MAXB + " bytes of text");
        return x;
      }
      if (x === null || t !== "object") return x;
      if (d >= MAXD) throw new RangeError("value nested deeper than " + MAXD + " levels");
      if (isArray(x)) {
        const out = [], n = x.length;
        for (let i = 0; i < n; i++) { const v = walk(x[i], d + 1, x, "" + i); put(out, i, v === undefined ? null : v); }
        return out;
      }
      const out = {}, ks = list || keys(x);
      for (let i = 0; i < ks.length; i++) {
        const k = ks[i], xv = x[k], kk = host ? wellFormed(k) : k;
        // Left out of the output (a listed key the object lacks): costs nothing.
        if (xv === undefined && !replacer) continue;
        bytes += k.length;
        if (bytes > MAXB) throw new RangeError("value holds more than " + MAXB + " bytes of text");
        const v = walk(xv, d + 1, x, k);
        if (v !== undefined) put(out, kk, v);
      }
      return out;
    };
    return walk(root, 0, { "": root }, "");
  };
  // Native JSON.stringify recurses without a limit, so it only ever sees a bounded snapshot.
  // A function replacer runs inside the snapshot. An array replacer becomes the spec's
  // property list, so excluded keys are never read (their getters never run).
  def(JSON, "stringify", { value: function stringify(v, r, s) {
    let fn, list;
    if (typeof r === "function") fn = r;
    else if (isArray(r)) {
      list = [];
      const seen = create(null);
      for (let i = 0, n = r.length; i < n; i++) {
        let k = r[i];
        if (typeof k === "object" && k !== null) k = unbox(k);
        if (typeof k === "number") k = Str(k);
        if (typeof k !== "string" || seen[k] === 1) continue;
        seen[k] = 1;
        put(list, list.length, k);
      }
    }
    // The list again: integer keys would otherwise come first in the copy's key order.
    return str(snapshot(v, fn, list), list, s);
  }, writable: true, configurable: true });
  // Boa 0.22 panics (a trap on the IC) on `"x".at(-1e19)`, and pads/repeats by allocating
  // the full length up front, up to 8 GB. Replace `at` and bound the length arguments.
  const SP = String.prototype, trunc = Math.trunc, MAXLEN = __MAXLEN__;
  def(SP, "at", { value: function at(index) {
    if (this == null) throw new TypeError("String.prototype.at called on null or undefined");
    const s = Str(this), n = s.length;
    let k = trunc(+index) || 0;
    if (k < 0) k += n;
    return k < 0 || k >= n ? undefined : s[k];
  }, writable: true, configurable: true });
  const bounded = (name, length) => {
    const f = SP[name];
    def(SP, name, { value: { [name](a, b) {
      const n = +a;
      if (length(Str(this), n) > MAXLEN) throw new RangeError(name + ": result longer than " + MAXLEN + " characters");
      return apply(f, this, [n, b]);
    } }[name], writable: true, configurable: true });
  };
  bounded("padStart", (s, n) => n);
  bounded("padEnd", (s, n) => n);
  bounded("repeat", (s, n) => s.length * n);
  // console.log / error text: the language's own text for primitives, functions and
  // errors (only `name` and `message`, cut short), bounded JSON for other objects.
  const big = (k, x) => typeof x === "bigint" ? Str(x) + "n" : x;
  const show = v => {
    const t = typeof v;
    if (t === "string") return v;
    if (t === "bigint") return Str(v) + "n";
    if (t === "function") { const n = v.name; return typeof n === "string" && n ? "[Function " + cut(n) + "]" : "[Function]"; }
    if (t !== "object" || v === null) return Str(v);
    if (apply(objToString, v, []) === "[object Error]") return errText(v);
    return str(snapshot(v, big));
  };
  return [snapshot, show];
})()"#;

/// Job queue with a ceiling. Boa's own executor runs promise jobs until none are left,
/// with no count and no budget check, so a self-rescheduling promise chain would spin
/// until the IC's instruction limit trapped the canister.
#[derive(Default)]
struct BoundedJobs {
    promise: RefCell<VecDeque<PromiseJob>>,
    generic: RefCell<VecDeque<GenericJob>>,
    /// Host async jobs: only dynamic `import()` creates one, and the idle module loader
    /// settles it on the first poll.
    asynk: RefCell<VecDeque<NativeAsyncJob>>,
    /// A timer or finalization job arrived (nothing in the sandbox should create one).
    dropped: Cell<bool>,
    ran: Cell<u64>,
    deadline: u64,
}

impl BoundedJobs {
    fn tick(&self) -> JsResult<()> {
        let n = self.ran.get() + 1;
        self.ran.set(n);
        if n > MAX_PROMISE_JOBS {
            return Err(JsNativeError::range().with_message(format!("script ran more than {MAX_PROMISE_JOBS} promise jobs")).into());
        }
        if n.is_multiple_of(16) {
            budget_error(self.deadline).map_err(|e| JsNativeError::range().with_message(e))?;
        }
        Ok(())
    }

    fn drain(&self, ctx: &mut Context) -> JsResult<()> {
        loop {
            let job = self.promise.borrow_mut().pop_front(); // borrow released before the job runs
            if let Some(job) = job {
                self.tick()?;
                job.call(ctx)?;
                continue;
            }
            let job = self.generic.borrow_mut().pop_front();
            if let Some(job) = job {
                self.tick()?;
                job.call(ctx)?;
                continue;
            }
            let job = self.asynk.borrow_mut().pop_front();
            if let Some(job) = job {
                self.tick()?;
                use futures::FutureExt;
                match job.call(&RefCell::new(&mut *ctx)).now_or_never() {
                    Some(r) => r?,
                    None => return Err(JsNativeError::typ().with_message("asynchronous host jobs are not supported").into()),
                };
                continue;
            }
            if self.dropped.get() {
                return Err(JsNativeError::typ().with_message("timer and finalization jobs are not supported").into());
            }
            return Ok(());
        }
    }
}

impl JobExecutor for BoundedJobs {
    fn enqueue_job(self: Rc<Self>, job: Job, _: &mut Context) {
        match job {
            Job::PromiseJob(p) => self.promise.borrow_mut().push_back(p),
            Job::GenericJob(g) => self.generic.borrow_mut().push_back(g),
            Job::AsyncJob(a) => self.asynk.borrow_mut().push_back(a),
            // Timers do not exist and FinalizationRegistry is removed; fail loudly if one appears.
            _ => self.dropped.set(true),
        }
    }

    fn run_jobs(self: Rc<Self>, ctx: &mut Context) -> JsResult<()> {
        let r = self.drain(ctx);
        if r.is_err() {
            self.promise.borrow_mut().clear();
            self.generic.borrow_mut().clear();
            self.asynk.borrow_mut().clear();
        }
        ctx.clear_kept_objects();
        r
    }
}

/// No local timezone, no `eval` / `new Function`.
struct Hooks;
impl HostHooks for Hooks {
    fn ensure_can_compile_strings(&self, _: Realm, _: &[JsString], _: &JsString, _: bool, _: &mut Context) -> JsResult<()> {
        Err(JsNativeError::eval().with_message("dynamic code evaluation is disabled").into())
    }
    fn local_timezone_offset_seconds(&self, _: i64) -> i32 {
        0
    }
    /// An ArrayBuffer larger than a run may hold is refused with a RangeError up front.
    fn max_buffer_size(&self, _: &mut Context) -> u64 {
        JS_MAX_MEMORY as u64
    }
}

/// Why the VM interrupt stopped a run.
#[derive(Clone, Copy)]
enum Stop {
    Budget,
    Memory,
}

/// Per-run resources, checked by the VM interrupt (vendor/boa_engine PATCHES.md) every 64
/// VM steps and every 4096 property accesses, so work inside one script turn, native
/// loops included, stops at the deadline and the memory cap with a clean error.
#[derive(Default)]
struct Meter {
    deadline: u64,
    /// Heap bytes this run held when it last suspended.
    own: Cell<usize>,
    /// Live-heap value that means "this run holds nothing", for the current poll.
    base: Cell<usize>,
    stop: Cell<Option<Stop>>,
}

/// Live heap bytes (counting allocator in lib.rs).
fn live() -> usize {
    // Tests share the counter across threads; only a thread that counts may read it.
    #[cfg(test)]
    if !crate::counting_alloc::COUNT_THIS_THREAD.with(|c| c.get()) {
        return 0;
    }
    crate::counting_alloc::live()
}

#[cfg(test)]
thread_local!(static TEST_BUDGET_CHECKS: Cell<u64> = const { Cell::new(u64::MAX) });

impl Meter {
    fn held(&self) -> usize {
        live().saturating_sub(self.base.get())
    }

    fn over_budget(&self) -> bool {
        // Natively the instruction counter is always 0: tests trip the deadline after N checks.
        #[cfg(test)]
        if TEST_BUDGET_CHECKS.with(|c| {
            c.set(c.get().saturating_sub(1));
            c.get() == 0
        }) {
            return true;
        }
        budget_error(self.deadline).is_err()
    }

    /// The VM interrupt: `true` stops the run uncatchably.
    fn interrupt(&self) -> bool {
        let stop = if self.over_budget() {
            Some(Stop::Budget)
        } else if self.held() > JS_MAX_MEMORY && {
            // Our own garbage may be what is over the cap; the shared heap is collected
            // only when a collection threshold is crossed.
            boa_gc::force_collect();
            self.held() > JS_MAX_MEMORY
        } {
            Some(Stop::Memory)
        } else {
            None
        };
        if stop.is_some() {
            self.stop.set(stop);
        }
        stop.is_some()
    }
}

/// Polls `f` with this run's heap measured from `meter`. Each poll after a suspension runs
/// in a new message execution, where nothing else runs until it returns, so the heap
/// growth during a poll is this run's, and what other runs did while it waited is not.
async fn metered<T>(meter: &Meter, f: impl std::future::Future<Output = T>) -> T {
    let mut f = std::pin::pin!(f);
    std::future::poll_fn(|cx| {
        meter.base.set(live().saturating_sub(meter.own.get()));
        let r = f.as_mut().poll(cx);
        meter.own.set(meter.held());
        r
    })
    .await
}

thread_local!(static ACTIVE_RUNS: Cell<u32> = const { Cell::new(0) });

/// One live JS run. Boa's GC heap is shared by every run, so a collection sweeps all of
/// their objects and bills whoever triggers it: only the last run to end collects. Dropping
/// counts as ending, so a run that traps after an await (ic-cdk drops its future) never
/// leaves its garbage behind once every run is done.
struct Active;

impl Active {
    fn enter() -> Self {
        ACTIVE_RUNS.with(|a| a.set(a.get() + 1));
        Active
    }
}

/// Ends one run; collects (and returns true) if it was the last.
fn leave() -> bool {
    let last = ACTIVE_RUNS.with(|a| {
        a.set(a.get().saturating_sub(1));
        a.get() == 0
    });
    if last {
        boa_gc::force_collect();
    }
    last
}

impl Drop for Active {
    fn drop(&mut self) {
        leave();
    }
}

/// Lowest native stack address the engine may reach from here (see `NATIVE_STACK`).
fn stack_floor() -> usize {
    let probe = 0u8;
    (std::hint::black_box(&raw const probe) as usize).saturating_sub(NATIVE_STACK)
}

#[derive(Trace, Finalize)]
struct Pending {
    /// `None` = icp.candid, `Some(method)` = icp.call
    method: Option<String>,
    canister: String,
    /// Plain data, no GC references. Kept as a `Value`: a round trip through text would
    /// meet serde_json's 128-level parse limit and turn deep arguments into `null`.
    #[unsafe_ignore_trace]
    args: Value,
    resolvers: ResolvingFunctions,
}

#[derive(Trace, Finalize)]
struct State {
    pending: Vec<Pending>,
    logs: Vec<String>,
    log_bytes: usize,
    principal: String,
    #[unsafe_ignore_trace]
    meter: Rc<Meter>,
    snapshot: Option<JsFunction>,
    show: Option<JsFunction>,
}

type Shared = Gc<GcRefCell<State>>;

/// Longest canister id text accepted (a principal's text form is at most 63 characters).
const MAX_ID_CHARS: usize = 64;

/// String argument `i`, refused before it is copied out of the engine if longer than `max`.
fn arg_string(args: &[JsValue], i: usize, max: usize, _: &mut Context) -> JsResult<String> {
    match args.get(i).and_then(JsValue::as_string) {
        Some(s) if s.len() > max => {
            Err(JsNativeError::range().with_message(format!("argument {} is longer than {max} characters", i + 1)).into())
        }
        Some(s) => Ok(s.to_std_string_escaped()),
        None => Err(JsNativeError::typ().with_message(format!("argument {} must be a string", i + 1)).into()),
    }
}

fn budget(st: &Shared) -> JsResult<()> {
    let deadline = st.borrow().meter.deadline;
    budget_error(deadline).map_err(|e| JsNativeError::range().with_message(e).into())
}

/// Bounded JS -> JSON conversion (see `SNAPSHOT_JS`).
fn to_json_safe(v: &JsValue, st: &Shared, ctx: &mut Context) -> JsResult<Value> {
    let snapshot = st.borrow().snapshot.clone();
    let plain = match snapshot {
        Some(f) => f.call(&JsValue::undefined(), &[v.clone(), JsValue::undefined(), JsValue::undefined(), JsValue::from(true)], ctx)?,
        None => return Err(JsNativeError::error().with_message("engine not initialised").into()),
    };
    let v = normalize_numbers(plain.to_json(ctx)?.unwrap_or(Value::Null));
    // The snapshot counts UTF-16 code units; the limit is in UTF-8 bytes, as elsewhere.
    match crate::bridge::text_bytes(&v) {
        n if n > MAX_VALUE_BYTES => {
            Err(JsNativeError::range().with_message(format!("value holds more than {MAX_VALUE_BYTES} bytes of text ({n})")).into())
        }
        _ => Ok(v),
    }
}

/// Text for `console.log` and error messages (see `show` in `SNAPSHOT_JS`).
fn display(v: &JsValue, st: &Shared, ctx: &mut Context) -> Option<String> {
    if let Some(s) = v.as_string() {
        return Some(s.to_std_string_escaped());
    }
    let show = st.borrow().show.clone()?;
    let s = show.call(&JsValue::undefined(), std::slice::from_ref(v), ctx).ok()?;
    s.as_string().map(|s| s.to_std_string_escaped())
}

fn console_fn(level: &'static str, state: &Shared) -> NativeFunction {
    NativeFunction::from_copy_closure_with_captures(
        move |_, args, st: &Shared, ctx| {
            budget(st)?;
            if st.borrow().log_bytes >= MAX_LOG_BYTES {
                return Ok(JsValue::undefined()); // cap reached: don't even format
            }
            let room = MAX_LOG_BYTES - st.borrow().log_bytes;
            let mut line = if level == "log" { String::new() } else { format!("[{level}] ") };
            for (i, a) in args.iter().enumerate() {
                if line.len() > room {
                    break; // the rest would be cut anyway: don't format it
                }
                if i > 0 {
                    line.push(' ');
                }
                line.push_str(&display(a, st, ctx).unwrap_or_else(|| "<unprintable value>".into()));
            }
            let mut s = st.borrow_mut();
            if let Some((line, used)) = log_line(s.log_bytes, line) {
                s.log_bytes = used;
                s.logs.push(line);
            }
            Ok(JsValue::undefined())
        },
        state.clone(),
    )
}

fn request_fn(is_call: bool, state: &Shared) -> NativeFunction {
    NativeFunction::from_copy_closure_with_captures(
        move |_, args, st: &Shared, ctx| {
            budget(st)?;
            let (name, max) = if is_call { ("call", 3) } else { ("candid", 1) };
            if args.len() > max {
                return Err(JsNativeError::typ().with_message(format!("icp.{name} takes at most {max} arguments")).into());
            }
            let full = || JsNativeError::range().with_message(format!("more than {MAX_PENDING} concurrent icp requests"));
            if st.borrow().pending.len() >= MAX_PENDING {
                return Err(full().into());
            }
            let canister = arg_string(args, 0, MAX_ID_CHARS, ctx)?;
            let (method, args) = if is_call {
                let method = arg_string(args, 1, crate::bridge::MAX_METHOD_LEN, ctx)?;
                let a = to_json_safe(&args.get(2).cloned().unwrap_or_default(), st, ctx)?;
                (Some(method), a)
            } else {
                (None, Value::Null)
            };
            let (promise, resolvers) = JsPromise::new_pending(ctx);
            let mut s = st.borrow_mut();
            // Again: converting the arguments ran script code (getters, toString), which may
            // have queued requests of its own.
            if s.pending.len() >= MAX_PENDING {
                return Err(full().into());
            }
            s.pending.push(Pending { method, canister, args, resolvers });
            Ok(promise.into())
        },
        state.clone(),
    )
}

fn new_context(state: &Shared) -> JsResult<Context> {
    let meter = state.borrow().meter.clone();
    let mut ctx = ContextBuilder::new()
        // Required: the default StdClock calls Instant::now(), which traps on wasm32.
        .clock(Rc::new(FixedClock::from_millis(now_ms())))
        .host_hooks(Rc::new(Hooks))
        .module_loader(Rc::new(IdleModuleLoader))
        .job_executor(Rc::new(BoundedJobs { deadline: meter.deadline, ..BoundedJobs::default() }))
        .instructions_remaining(MAX_VM_STEPS)
        .interrupt(Rc::new(move || meter.interrupt()))
        .build()?;
    let limits = ctx.runtime_limits_mut();
    limits.set_recursion_limit(RECURSION_LIMIT);
    limits.set_stack_size_limit(STACK_SIZE_LIMIT);
    limits.set_backtrace_limit(8);

    let principal = NativeFunction::from_copy_closure_with_captures(
        |_, args, st: &Shared, _| match args {
            [] => Ok(JsString::from(st.borrow().principal.as_str()).into()),
            _ => Err(JsNativeError::typ().with_message("icp.principal takes no arguments").into()),
        },
        state.clone(),
    );
    let icp = ObjectInitializer::new(&mut ctx)
        .function(request_fn(true, state), js_string!("call"), 3)
        .function(request_fn(false, state), js_string!("candid"), 1)
        .function(principal, js_string!("principal"), 0)
        .build();
    ctx.register_global_property(js_string!("icp"), icp, Attribute::READONLY | Attribute::NON_ENUMERABLE)?;

    let mut console = ObjectInitializer::new(&mut ctx);
    for level in ["log", "info", "warn", "error", "debug"] {
        console.function(console_fn(level, state), JsString::from(level), 0);
    }
    let console = console.build();
    ctx.register_global_property(js_string!("console"), console, Attribute::all())?;
    ctx.register_global_callable(js_string!("print"), 0, console_fn("log", state))?;

    // Determinism: no RNG, nothing that can wait on wall-clock timers.
    let math = ctx.global_object().get(js_string!("Math"), &mut ctx)?;
    if let Some(m) = math.as_object() {
        m.delete_property_or_throw(js_string!("random"), &mut ctx)?;
    }
    let global = ctx.global_object();
    for name in ["Atomics", "SharedArrayBuffer", "WeakRef", "FinalizationRegistry"] {
        global.delete_property_or_throw(JsString::from(name), &mut ctx)?;
    }
    // Boa's `Symbol.for` registry is process-wide and never shrinks: each run could add
    // keys that outlive it.
    let symbol = global.get(js_string!("Symbol"), &mut ctx)?;
    if let Some(sym) = symbol.as_object() {
        sym.delete_property_or_throw(js_string!("for"), &mut ctx)?;
        sym.delete_property_or_throw(js_string!("keyFor"), &mut ctx)?;
    }

    let src = SNAPSHOT_JS.replace("__MAXD__", &MAX_VALUE_DEPTH.to_string()).replace("__MAXN__", &MAX_VALUE_NODES.to_string())
        .replace("__MAXB__", &MAX_VALUE_BYTES.to_string()).replace("__MAXLEN__", &MAX_STRING_LEN.to_string());
    let pair = ctx.eval(Source::from_bytes(src.as_bytes()))?;
    let pair = pair.as_object().ok_or_else(|| JsNativeError::error().with_message("prelude"))?;
    let snapshot = pair.get(0, &mut ctx)?.as_object().and_then(JsFunction::from_object);
    let show = pair.get(1, &mut ctx)?.as_object().and_then(JsFunction::from_object);
    let mut s = state.borrow_mut();
    (s.snapshot, s.show) = (snapshot, show);
    drop(s);
    Ok(ctx)
}

impl Stop {
    fn message(self) -> String {
        match self {
            Stop::Budget => "policy: instruction budget exceeded while the script was running".into(),
            Stop::Memory => format!("policy: script holds more than {} MiB of heap", JS_MAX_MEMORY >> 20),
        }
    }
}

/// Err once the run has been stopped uncatchably (Boa can drop such an error inside a job).
fn stopped(st: &Shared, ctx: &Context) -> Result<(), String> {
    match st.borrow().meter.stop.get() {
        Some(stop) => Err(stop.message()),
        None if ctx.instructions_remaining() == 0 => Err(format!("policy: script ran more than {MAX_VM_STEPS} VM steps")),
        None => Ok(()),
    }
}

/// Bounded text for a thrown value: never walks an error's `errors`/`cause`, and never
/// copies more than `MAX_ERROR_BYTES`.
fn error_string(err: JsError, st: &Shared, ctx: &mut Context) -> String {
    let mut msg = error_text(err, st, ctx);
    truncate(&mut msg, MAX_ERROR_BYTES);
    msg
}

fn error_text(err: JsError, st: &Shared, ctx: &mut Context) -> String {
    let text = stopped(st, ctx).err().unwrap_or_else(|| match (err.as_engine(), err.as_native(), err.as_opaque()) {
        // A `PanicError` names the step Boa assumed infallible; its root cause says why
        // (e.g. the native stack limit).
        (Some(e), ..) => match std::iter::successors(Some(e as &dyn std::error::Error), |c| c.source()).skip(1).last() {
            Some(root) => format!("{e} ({root})"),
            None => e.to_string(),
        },
        (_, Some(native), _) => {
            // Host functions and the snapshot helper report positions that mean nothing to
            // the script author: " (native)" / " (unknown at :L:C)".
            let msg = native.to_string();
            match msg.rfind(" (unknown at ").or_else(|| msg.rfind(" (native)")) {
                Some(i) if msg.ends_with(')') => msg[..i].to_string(),
                _ => msg,
            }
        }
        (_, _, Some(v)) => {
            let is_error = v.as_object().is_some_and(|o| o.is::<boa_engine::builtins::error::Error>());
            match display(v, st, ctx) {
                Some(text) if is_error => text,
                Some(text) => format!("Uncaught {text}"),
                None => "Uncaught <unserializable value>".into(),
            }
        }
        _ => "Uncaught error".into(),
    });
    // Formatting runs script code (toJSON, getters), which the interrupt may have stopped.
    stopped(st, ctx).err().unwrap_or(text)
}

pub async fn run<B: HostBridge>(code: &str, bridge: &B) -> Result<Value, String> {
    let _active = Active::enter(); // dropped last, after the engine
    let meter = Rc::new(Meter { deadline: bridge.instruction_deadline(), ..Meter::default() });
    metered(&meter.clone(), run_metered(code, bridge, meter)).await
}

async fn run_metered<B: HostBridge>(code: &str, bridge: &B, meter: Rc<Meter>) -> Result<Value, String> {
    // Declared before the Context so it is dropped after it.
    let state: Shared = Gc::new(GcRefCell::new(State {
        pending: vec![],
        logs: vec![],
        log_bytes: 0,
        principal: bridge.principal(),
        meter,
        snapshot: None,
        show: None,
    }));
    let mut ctx = new_context(&state).map_err(|e| format!("engine init: {e}"))?;
    let out = drive(code, bridge, &state, &mut ctx).await;
    for line in std::mem::take(&mut state.borrow_mut().logs) {
        bridge.log(line);
    }
    out
}

/// Refuses source nested deeper than `MAX_NESTING` before Boa parses it, since the parser
/// and compiler recurse natively once per level and a stack overflow traps. A level is an
/// open `( [ { ${`, a right-associative operator (`=`, `=>`, `?`, `**`) or a statement that
/// nests without brackets (`if`, `for`, `while`, `with`, `do`, `extends`; two labels make one)
/// since the statement or the last `,` began in the enclosing brackets, a prefix operator in
/// an unbroken run (`!!!x`, `typeof void x`), or 16 other operators (`a + b + ...` also
/// nests, more cheaply). An `else` continues its `if` statement. Strings, comments, regex
/// literals and template text are skipped.
// ponytail: a lexical scan, not Boa's grammar. `/` after `)` or `}` is taken as a regex
// exactly when the grammar usually makes it one; source written to confuse that guess can
// hide nesting from the scan (the README lists this).
fn check_nesting(code: &str) -> Result<(), String> {
    let b = code.as_bytes();
    let at = |i: usize| b.get(i).copied().unwrap_or(0);
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80;
    // Open levels: (kind, operators counted in the enclosing level when it opened).
    // Kinds: b'(' b'[' b'{', b'c' a `(` after if/while/for/with, b'$' `${`, b'`' template text.
    let mut open: Vec<(u8, usize)> = vec![];
    // Operators in sixteenths of a level, in this bracket level (`ops`) and enclosing ones.
    let (mut ops, mut run, mut outer) = (0usize, 0usize, 0usize);
    let over = |outer: usize, ops: usize, run: usize, depth: usize| (outer + ops) / 16 + run + depth > MAX_NESTING;
    let mut regex_ok = true; // a `/` here starts a regex literal
    // A statement can end at a line break after `ended` (an operand or closer), so a
    // word starting the next line resets the operator count, as `;` does.
    let (mut ended, mut fresh) = (false, false);
    let mut prev_word: &[u8] = b"";
    let mut dot = false; // the last token was `.` or `?.`: a word here is a property name
    // Whether the next token after `j`, past blanks and comments, is the word `else`.
    let else_next = |mut j: usize| loop {
        match (at(j), at(j + 1)) {
            (b' ' | b'\t' | b'\r' | b'\n', _) => j += 1,
            (b'/', b'/') => j = code[j..].find('\n').map_or(b.len(), |k| j + k),
            (b'/', b'*') => j = code[j + 2..].find("*/").map_or(b.len(), |k| j + 2 + k + 2),
            _ => break b.get(j..).is_some_and(|r| r.starts_with(b"else")) && !word(at(j + 4)),
        }
    };
    let deep = || Err(format!("script nests deeper than {MAX_NESTING} levels (brackets and chained operators)"));
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if open.last().is_some_and(|l| l.0 == b'`') {
            match c {
                b'\\' => i += 1,
                b'`' => {
                    open.pop();
                    (regex_ok, ended) = (false, true);
                }
                b'$' if at(i + 1) == b'{' => {
                    open.push((b'$', ops));
                    (outer, ops, run, regex_ok) = (outer + ops, 0, 0, true);
                    i += 1;
                }
                _ => {}
            }
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        let mut operand = false; // this token ends an operand: `/` after it divides
        match c {
            b' ' | b'\t' | b'\r' => continue,
            b'\n' => {
                fresh |= ended;
                continue;
            }
            b'/' if at(i) == b'/' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if at(i) == b'*' => {
                i = code[i + 1..].find("*/").map_or(b.len(), |j| i + 1 + j + 2);
                continue;
            }
            b'\'' | b'"' => {
                while i < b.len() && b[i] != c && b[i] != b'\n' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
                operand = true;
            }
            b'/' if regex_ok => {
                let mut class = false;
                while i < b.len() && b[i] != b'\n' && (class || b[i] != b'/') {
                    match b[i] {
                        b'\\' => i += 1,
                        b'[' => class = true,
                        b']' => class = false,
                        _ => {}
                    }
                    i += 1;
                }
                i += 1;
                while i < b.len() && word(b[i]) {
                    i += 1;
                }
                operand = true;
            }
            b'`' => open.push((b'`', 0)),
            b'(' | b'[' | b'{' => {
                let kind = if c == b'(' && matches!(prev_word, b"if" | b"while" | b"for" | b"with") { b'c' } else { c };
                ops += 16 * run; // `!!(` keeps counting the prefix run inside
                open.push((kind, ops));
                (outer, ops, run) = (outer + ops, 0, 0);
            }
            b')' | b']' | b'}' => {
                if let Some((kind, saved)) = open.pop() {
                    (outer, ops) = (outer.saturating_sub(saved), saved);
                    if kind == b'$' {
                        prev_word = b"";
                        continue; // back in template text
                    }
                    // `if (x) /re/` vs `(a) / b`; after a block `}` a statement starts. The
                    // statement after `if (x)` continues on the next line.
                    regex_ok = kind == b'c' || c == b'}';
                    (run, ended, fresh, dot) = (0, kind != b'c', c == b'}', false);
                    prev_word = b"";
                    continue;
                }
            }
            b';' if else_next(i) => {} // `if (a) x; else if (b) ...` still nests
            b',' | b';' => ops = 0,
            b':' if !prev_word.is_empty() => ops += 8, // `a: b: c: x` nests (also object keys)
            b'?' if matches!(at(i), b'.' | b'?') => {
                ops += 1; // `?.`, `??`
                i += 1;
            }
            b'?' => ops += 16,
            b'*' if at(i) == b'*' => {
                ops += 16;
                i += 1;
            }
            b'=' if at(i) == b'>' || (at(i) != b'=' && !matches!(at(start.wrapping_sub(1)), b'=' | b'!')) => {
                ops += 16;
                i += usize::from(at(i) == b'>');
            }
            b'=' => {
                while at(i) == b'=' {
                    i += 1;
                }
            }
            b'!' if at(i) == b'=' => {
                while at(i) == b'=' {
                    i += 1;
                }
            }
            b'!' | b'~' => run += 1,
            b'+' | b'-' if regex_ok => {
                run += 1;
                i += usize::from(at(i) == c);
            }
            b'+' | b'-' if at(i) == c => {
                i += 1;
                operand = true; // postfix ++/--
            }
            _ if word(c) => {
                while i < b.len() && word(b[i]) {
                    i += 1;
                }
                let w = &b[start..i];
                if fresh && !matches!(w, b"in" | b"instanceof" | b"of" | b"else") {
                    ops = 0;
                }
                // After `.` a word is a property name, never a keyword (`o.return / x`).
                if !dot && matches!(w, b"typeof" | b"void" | b"delete" | b"await" | b"new" | b"yield") {
                    run += 1;
                } else {
                    run = 0;
                }
                if !dot && matches!(w, b"if" | b"for" | b"while" | b"with" | b"do" | b"extends") {
                    ops += 16;
                }
                prev_word = if dot { b"_" } else { w }; // `o.if(x) / y` is a call
                let keyword = !dot
                    && matches!(w, b"return" | b"typeof" | b"instanceof" | b"in" | b"of" | b"new" | b"delete" | b"void"
                        | b"throw" | b"case" | b"do" | b"else" | b"yield" | b"await" | b"extends");
                // `of`, `yield` and `await` can be identifiers too: taking `/` after them as
                // division can only over-count.
                regex_ok = keyword && !matches!(w, b"of" | b"yield" | b"await");
                // A line break after `return` or `yield` ends the statement.
                (ended, fresh, dot) = (!keyword || matches!(w, b"return" | b"yield"), false, false);
                if over(outer, ops, run, open.len()) {
                    return deep();
                }
                continue;
            }
            _ => ops += 1, // binary operators, `.`, `:`
        }
        if operand {
            run = 0;
        }
        prev_word = b"";
        (regex_ok, ended, fresh) = (!operand, operand, false);
        dot = c == b'.' || (c == b'?' && at(start + 1) == b'.');
        if over(outer, ops, run, open.len()) {
            return deep();
        }
    }
    Ok(())
}

async fn drive<B: HostBridge>(code: &str, bridge: &B, st: &Shared, ctx: &mut Context) -> Result<Value, String> {
    check_nesting(code)?;
    ctx.set_stack_floor(stack_floor());
    // Leading newline keeps `//` comments on the last line from eating the wrapper.
    let wrapped = format!("(async () => {{\n{code}\n}})()");
    let v = ctx.eval(Source::from_bytes(wrapped.as_bytes())).map_err(|e| error_string(e, st, ctx))?;
    let promise = v
        .as_object()
        .and_then(|o| JsPromise::from_object(o).ok())
        .ok_or("script must not escape its async wrapper")?;

    for round in 0..=MAX_ROUNDS {
        let jobs = ctx.run_jobs();
        stopped(st, ctx)?;
        jobs.map_err(|e| error_string(e, st, ctx))?;
        check_budget(bridge)?;
        match promise.state() {
            PromiseState::Fulfilled(v) => {
                // Converting the result can run script code (toJSON, getters) that queues requests.
                let out = to_json_safe(&v, st, ctx).map_err(|e| error_string(e, st, ctx));
                stopped(st, ctx)?;
                return match st.borrow().pending.len() {
                    // Not sent: the script finished first. Say so instead of reporting success.
                    0 => out,
                    n => Err(format!("{n} icp request(s) were started but never awaited, so they were not sent")),
                };
            }
            PromiseState::Rejected(e) => {
                let mut msg = error_string(JsError::from_opaque(e), st, ctx);
                if let n @ 1.. = st.borrow().pending.len() {
                    msg.push_str(&format!(" ({n} icp request(s) were started but never sent)"));
                }
                return Err(msg);
            }
            PromiseState::Pending => {}
        }
        if round == MAX_ROUNDS {
            // Refuse before sending, so this error never follows messages that went out.
            return Err(format!("script exceeded {MAX_ROUNDS} host round-trips; batch calls with Promise.all"));
        }
        // Never hold the state borrow while JS runs: resolvers re-enter native fns.
        let batch = std::mem::take(&mut st.borrow_mut().pending);
        if batch.len() > MAX_PENDING {
            return Err(format!("more than {MAX_PENDING} concurrent icp requests"));
        }
        if batch.is_empty() {
            // Boa drops uncatchable job errors (loop/recursion limits) after an await,
            // leaving the promise pending, so we cannot tell the two cases apart.
            return Err("script stopped before finishing: a runtime limit (loop iterations, recursion, stack) \
                        was hit after an await, or it awaits a promise that never settles"
                .into());
        }
        let replies = futures::future::join_all(batch.iter().map(|p| async move {
            match &p.method {
                None => bridge.candid(&p.canister).await.map(Value::String),
                Some(m) => bridge.call(&p.canister, m, p.args.clone()).await,
            }
        }))
        .await;
        // Resumed in a new message execution, on a fresh stack.
        ctx.set_stack_floor(stack_floor());
        for (p, reply) in batch.iter().zip(replies) {
            // Each reply is within the value budget; several of them are not.
            check_budget(bridge)?;
            let (f, arg) = match reply {
                Ok(j) => (&p.resolvers.resolve, JsValue::from_json(&j, ctx).map_err(|e| error_string(e, st, ctx))?),
                Err(msg) => (&p.resolvers.reject, JsNativeError::error().with_message(msg).into_opaque(ctx).into()),
            };
            f.call(&JsValue::undefined(), &[arg], ctx).map_err(|e| error_string(e, st, ctx))?;
        }
    }
    unreachable!("the loop returns on its last round")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::mock::{MockBridge, YieldBridge};
    use futures::executor::block_on;
    use serde_json::json;

    fn js(code: &str) -> Result<Value, String> {
        block_on(run(code, &MockBridge::default()))
    }

    #[test]
    fn determinism_hardening() {
        assert_eq!(js("return typeof Math.random").unwrap(), json!("undefined"));
        assert_eq!(js("return Date.now()").unwrap(), json!(0));
        assert_eq!(js("return new Date().getTimezoneOffset()").unwrap(), json!(0));
        assert_eq!(js("return [typeof Atomics, typeof SharedArrayBuffer, typeof WeakRef].join()").unwrap(), json!("undefined,undefined,undefined"));
        assert!(js("return eval('1')").unwrap_err().contains("disabled"));
        assert!(js("return new Function('return 1')()").unwrap_err().contains("disabled"));
    }

    #[test]
    fn promise_all_batches_in_one_round() {
        let b = MockBridge::default();
        let code = "const ids = [1,2,3].map(i => icp.call('ryjl3-tyaaa-aaaaa-aaaba-cai', 'icrc1_balance_of', {owner: 'aaaaa-aa', i}));
                    const rs = await Promise.all(ids); return rs.map(r => r.balance);";
        assert_eq!(block_on(run(code, &b)).unwrap(), json!([100, 200, 300]));
        assert_eq!(b.calls.borrow().len(), 3);
    }

    #[test]
    fn values_and_errors() {
        assert_eq!(js("return {a: 1.5, b: [true, null], c: 'x', d: 2**31}").unwrap(), json!({ "a": 1.5, "b": [true, null], "c": "x", "d": 2147483648u64 }));
        assert_eq!(js("1 + 1").unwrap(), Value::Null); // no `return` => undefined => null
        assert!(js("throw new TypeError('boom')").unwrap_err().contains("TypeError: boom"));
        assert_eq!(js("throw {code: 7}").unwrap_err(), "Uncaught {\"code\":7}");
        assert!(js("return 1n").is_err()); // BigInt is not JSON
        assert!(js("await new Promise(() => {})").unwrap_err().contains("never settle"));
        assert!(js("icp.call(1, 2)").unwrap_err().contains("must be a string"));
        assert!(js("})(); (() => {").is_err());
    }

    #[test]
    fn deep_and_huge_values_are_rejected_not_overflowing() {
        // Each of these used to recurse unboundedly in Boa's to_json / JSON.stringify.
        let deep = "let o = 1; for (let i = 0; i < 100000; i++) o = [o];";
        assert!(js(&format!("{deep} return o")).unwrap_err().contains("nested deeper"));
        assert!(js(&format!("{deep} return JSON.stringify(o)")).unwrap_err().contains("nested deeper"));
        assert!(js(&format!("{deep} console.log(o); return 1")).is_ok());
        assert!(js(&format!("{deep} return await icp.call('ryjl3-tyaaa-aaaaa-aaaba-cai', 'icrc1_balance_of', o)")).unwrap_err().contains("nested deeper"));
        assert!(js(&format!("{deep} throw o")).unwrap_err().contains("unserializable"));
        // A getter cannot return a shallow value to the check and a deep one later.
        let sneaky = "let n = 0; const o = { get x() { n++; let v = 1; if (n > 1) for (let i = 0; i < 100000; i++) v = [v]; return v; } }; return o;";
        assert_eq!(js(sneaky).unwrap(), json!({ "x": 1 }));
        // JSON semantics preserved.
        assert_eq!(js("return JSON.stringify({a: [1, undefined, () => 1], d: new Date(0)})").unwrap(), json!("{\"a\":[1,null,null],\"d\":\"1970-01-01T00:00:00.000Z\"}"));
        assert!(js("return {n: 1n}").unwrap_err().contains("BigInt"));
    }

    #[test]
    fn promise_chains_are_bounded() {
        let e = js("const f = () => Promise.resolve().then(f); return f();").unwrap_err();
        assert!(e.contains("promise jobs"), "{e}");
        // Ordinary async code is unaffected.
        assert_eq!(js("let n = 0; for (let i = 0; i < 2000; i++) n += await Promise.resolve(1); return n;").unwrap(), json!(2000));
    }

    #[test]
    fn stringify_replacer_and_size_limits() {
        // A replacer cannot smuggle an unbounded value into the native serializer.
        let deep = "let d = 1; for (let i = 0; i < 100000; i++) d = [d];";
        assert!(js(&format!("{deep} return JSON.stringify({{a: 1}}, (k, v) => k === 'a' ? d : v)")).unwrap_err().contains("nested deeper"));
        // Replacer semantics are preserved: functions, key-filter arrays, `this`, toJSON-then-replacer.
        assert_eq!(js("return JSON.stringify({a: 1, b: 2, c: [1, 2]}, (k, v) => k === 'b' ? undefined : v)").unwrap(), json!("{\"a\":1,\"c\":[1,2]}"));
        assert_eq!(js("return JSON.stringify({a: 1, b: 2}, ['a'])").unwrap(), json!("{\"a\":1}"));
        assert_eq!(js("return JSON.stringify({n: 5n}, (k, v) => typeof v === 'bigint' ? v.toString() : v)").unwrap(), json!("{\"n\":\"5\"}"));
        assert_eq!(js("return JSON.stringify({a: 1}, null, 1)").unwrap(), json!("{\n \"a\": 1\n}"));
        // Every element counts, primitives included.
        assert!(js("return new Array(900000).fill(0)").unwrap_err().contains("elements"));
        assert_eq!(js("return new Array(1000).fill(0).length").unwrap(), json!(1000));
        // One big string referenced many times is small in the engine, huge when copied out.
        assert!(js("const s = 'x'.repeat(1000000); return new Array(300).fill(s)").unwrap_err().contains("bytes of text"));
    }

    #[test]
    fn big_js_integers_pass_as_nat() {
        let b = MockBridge::default();
        block_on(run("await icp.call('ryjl3-tyaaa-aaaaa-aaaba-cai', 'icrc1_balance_of', {owner: 'aaaaa-aa', n: 3000000000})", &b)).unwrap();
        assert_eq!(b.calls.borrow()[0].2["n"], json!(3_000_000_000u64));
    }

    #[test]
    fn console_output_is_capped_while_running() {
        let b = MockBridge::default();
        block_on(run("const s = 'x'.repeat(100000); for (let i = 0; i < 1000; i++) console.log(s); return 1", &b)).unwrap();
        let total: usize = b.logs.take().iter().map(|l| crate::bridge::log_cost(l)).sum();
        assert!(total <= MAX_LOG_BYTES, "{total}");
    }

    #[test]
    fn snapshot_counts_object_key_bytes_against_limit() {
        let err = js(
            "const o = {}; const k = 'k'.repeat(60000); \
             for (let i = 0; i < 100; i++) o[i + k] = 1; \
             return o;",
        )
        .unwrap_err();
        assert!(err.contains("bytes of text"), "{err}");
    }

    #[test]
    fn audit_round_two_js_fixes() {
        const L: &str = "ryjl3-tyaaa-aaaaa-aaaba-cai";
        // A request the script never awaited was never sent: say so instead of "ok".
        let e = js(&format!("icp.call('{L}', 'icrc1_decimals', null); return 1")).unwrap_err();
        assert!(e.contains("never awaited"), "{e}");
        // Work spread over many calls is stopped by the VM step cap, uncatchably.
        let e = js("function f() { for (let i = 0; i < 900000; i++) {} } \
                    try { for (let j = 0; j < 1000; j++) f(); } catch (_) {} return 1")
            .unwrap_err();
        assert!(e.contains("VM steps"), "{e}");
        // Boxed primitives serialise as their values.
        assert_eq!(js("return [new Number(1), Object('s'), new Boolean(false)]").unwrap(), json!([1, "s", false]));
        // The process-wide Symbol registry is gone.
        assert_eq!(js("return [typeof Symbol.for, typeof Symbol.keyFor]").unwrap(), json!(["undefined", "undefined"]));
        // Over-long ids and method names are refused before they are copied out.
        let e = js("return await icp.call('x'.repeat(100000), 'm', null)").unwrap_err();
        assert!(e.contains("longer than"), "{e}");
        // Text is measured in UTF-8 bytes, like the other runtimes: 1.5M three-byte chars.
        let e = js("const r = '\\u20ac'.repeat(750000); return r + r").unwrap_err();
        assert!(e.contains("bytes of text"), "{e}");
        // Arguments 127 levels deep are sent as they are, not replaced by null.
        let b = MockBridge::default();
        let deep = "let a = 1; for (let i = 0; i < 126; i++) a = [a]; return await icp.call('ryjl3-tyaaa-aaaaa-aaaba-cai', 'echo', a)";
        assert!(block_on(run(deep, &b)).is_ok());
        assert!(b.calls.borrow().iter().all(|c| !c.2.is_null()), "{:?}", b.calls.borrow().len());
    }

    #[test]
    fn string_at_negative_overflow_returns_undefined_without_panicking() {
        assert_eq!(js("return 'x'.at(-1e19) ?? null").unwrap(), json!(null));
        assert_eq!(js("return ['abc'.at(-1), 'abc'.at(0), 'abc'.at(1.7), 'abc'.at(NaN), 'abc'.at(3) ?? null]").unwrap(), json!(["c", "a", "b", "a", null]));
        assert_eq!(js("return ['ab'.padStart(4, '-'), 'ab'.padEnd(3), 'ab'.repeat(2)]").unwrap(), json!(["--ab", "ab ", "abab"]));
        for code in ["'x'.padStart(2 ** 31)", "'x'.padEnd(1e300)", "'xy'.repeat(2 ** 30)"] {
            let e = js(&format!("return {code}")).unwrap_err();
            assert!(e.contains("RangeError"), "{code}: {e}");
        }
    }

    #[test]
    fn pending_and_round_limits_are_enforced() {
        let e = js(
            "const a = []; for (let i = 0; i < 65; i++) a.push(icp.call('ryjl3-tyaaa-aaaaa-aaaba-cai', 'm', null)); \
             return await Promise.all(a);",
        )
        .unwrap_err();
        assert!(e.contains("concurrent icp requests"), "{e}");

        let rounds = js(
            "let x = 0; for (let i = 0; i < 65; i++) x = (await icp.call('ryjl3-tyaaa-aaaaa-aaaba-cai', 'm', null)).balance; \
             return x;",
        )
        .unwrap_err();
        assert!(rounds.contains("host round-trips"), "{rounds}");

        let err = js("throw new Error('plain');").unwrap_err();
        assert!(err.contains("Error: plain"), "{err}");
    }

    const L: &str = "ryjl3-tyaaa-aaaaa-aaaba-cai";

    /// Runs `f` with this thread's allocations counted, so the per-run memory cap applies.
    /// Python tests read the same process-wide counter, so take turns with them.
    fn counted<T>(f: impl FnOnce() -> T) -> T {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _turn = crate::counting_alloc::test_serial::Turn::take();
        let _lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = crate::counting_alloc::COUNT_THIS_THREAD.with(|c| c.replace(true));
        let r = f();
        crate::counting_alloc::COUNT_THIS_THREAD.with(|c| c.set(prev));
        r
    }

    /// Runs `code` with the instruction deadline reported as passed after `checks` interrupt polls.
    fn js_with_deadline_after(checks: u64, code: &str) -> Result<Value, String> {
        TEST_BUDGET_CHECKS.with(|c| c.set(checks));
        let r = js(code);
        TEST_BUDGET_CHECKS.with(|c| c.set(u64::MAX));
        r
    }

    /// A bridge whose calls never complete, like an inter-canister call still in flight.
    struct NeverBridge;
    impl HostBridge for NeverBridge {
        async fn candid(&self, _: &str) -> Result<String, String> {
            std::future::pending().await
        }
        async fn call(&self, _: &str, _: &str, _: Value) -> Result<Value, String> {
            std::future::pending().await
        }
        fn principal(&self) -> String {
            "aaaaa-aa".into()
        }
        fn log(&self, _: String) {}
    }

    // Finding 59: the instruction deadline stops work inside one script turn, uncatchably,
    // in VM code and in native loops; the per-frame loop limit is gone.
    #[test]
    fn deadline_interrupts_vm_and_native_loops() {
        let e = js_with_deadline_after(1000, "try { while (true) {} } catch (_) {} return 1").unwrap_err();
        assert!(e.contains("instruction budget exceeded while the script was running"), "{e}");
        // One VM step, billions of property reads: polled from property access.
        let e = js_with_deadline_after(200, "try { Array.prototype.indexOf.call({length: 2 ** 53 - 1}, 1) } catch (_) {} return 1").unwrap_err();
        assert!(e.contains("instruction budget"), "{e}");
        let e = js_with_deadline_after(200, "try { Array.prototype.sort.call({length: 2 ** 32 - 1}) } catch (_) {} return 1").unwrap_err();
        assert!(e.contains("instruction budget"), "{e}");
        // Stopped after an await too, with the same message.
        let e = js_with_deadline_after(1000, &format!("await icp.call('{L}', 'm', null); try {{ for (;;) {{}} }} catch (_) {{}} return 1")).unwrap_err();
        assert!(e.contains("instruction budget"), "{e}");
        // Work that used to hit the 1M per-frame loop limit.
        assert_eq!(js("return 'ab'.repeat(1100000).length").unwrap(), json!(2200000));
        assert_eq!(js("return 'a'.repeat(1 << 24).length").unwrap(), json!(1 << 24));
        assert_eq!(js("let n = 0; for (let i = 0; i < 600000; i++) n++; for (let i = 0; i < 600000; i++) n++; return n").unwrap(), json!(1200000));
        // The deadline can land inside steps Boa treats as infallible (`expect`): still a clean stop.
        let spread = "const src = {}; for (let i = 0; i < 2000; i++) src['k' + i] = i; for (;;) { const c = {...src} }";
        for n in (300..700).step_by(20) {
            let e = js_with_deadline_after(n, spread).unwrap_err();
            assert!(e.contains("instruction budget"), "{n}: {e}");
        }
        let e = js_with_deadline_after(1000, "throw { toJSON() { for (;;) {} } }").unwrap_err();
        assert!(e.contains("instruction budget"), "{e}");
        let e = js_with_deadline_after(1000, "async function* g() { for (;;) {} } for await (const v of g()) {}").unwrap_err();
        assert!(e.contains("instruction budget"), "{e}");
        // Finding 70: argument lists from a script-chosen length are not pre-sized.
        for code in ["Reflect.apply(function () {}, null, {length: 2 ** 45})", "(function () {}).apply(null, {length: 2 ** 45})",
                     "Reflect.construct(function () {}, {length: 2 ** 45})",
                     "Reflect.ownKeys(new Proxy({}, {ownKeys: () => new Proxy({}, {get: (_, k) => k === 'length' ? 2 ** 45 : 'k'})}))"] {
            let r = js_with_deadline_after(200, &format!("try {{ {code} }} catch (e) {{ return String(e) }} return 1"));
            assert!(r.as_ref().is_err_and(|e| e.contains("instruction budget")), "{code}: {r:?}");
        }
    }

    /// Interrupt polls `code` makes.
    fn polls(code: &str) -> u64 {
        TEST_BUDGET_CHECKS.with(|c| c.set(u64::MAX));
        js(code).unwrap();
        u64::MAX - TEST_BUDGET_CHECKS.with(Cell::get)
    }

    // Finding 59: the default sort comparator runs no checked step, yet the merge polls.
    #[test]
    fn default_sort_polls_the_interrupt() {
        let prep = polls("const a = new Array(1 << 16).fill(1); return 1");
        let copy = polls("const a = new Array(1 << 16).fill(1); a.slice(); return 1") - prep;
        let sort = polls("const a = new Array(1 << 16).fill(1); a.sort(); return 1") - prep;
        // Reading and writing back the elements polls about as often as a copy (3x); the
        // 16 merge passes add one poll per 4096 element moves.
        assert!(sort > 6 * copy, "{sort} vs {copy}");
    }

    // Findings 70, 71, R24: memory is capped per run, inside native loops and across awaits,
    // with a clean error instead of a trap.
    #[test]
    fn per_run_memory_cap() {
        counted(|| {
            for code in [
                "return Array.prototype.join.call({length: 2 ** 31}).length",
                "return Array.prototype.toSorted.call({length: 2 ** 29}).length",
                "return Array.prototype.toReversed.call({length: 2 ** 29}).length",
                "return String.raw({raw: {length: 2 ** 31}}).length",
                "let f = function () {}; for (let i = 0; i < 100000; i++) f = f.bind(null); return 1",
            ] {
                let e = js(&format!("try {{ {code} }} catch (e) {{ return 'caught ' + e }}")).unwrap_err();
                assert!(e.contains("MiB of heap"), "{code}: {e}");
            }
            let held = |n: usize| {
                let b = YieldBridge(MockBridge::default());
                let code = format!(
                    "const keep = []; for (let r = 0; r < 3; r++) {{ await icp.call('{L}', 'm', null); \
                     for (let i = 0; i < {n}; i++) keep.push('x'.repeat(1 << 20) + i); }} return keep.length"
                );
                block_on(run(&code, &b))
            };
            assert!(held(60).unwrap_err().contains("MiB of heap"));
            // Stopped while the thrown value is formatted: the policy error, not "unserializable".
            let e = js("throw { toJSON() { globalThis.k = []; for (let i = 0; i < 200; i++) k.push('x'.repeat(1 << 20) + i); return 1 } }").unwrap_err();
            assert!(e.contains("MiB of heap"), "{e}");
            assert_eq!(held(3).unwrap(), json!(9));
            // ArrayBuffers larger than a run may hold are a RangeError up front.
            // (Boa's own limit is 1.5 GiB, so this size tests the hook.)
            assert_eq!(js("try { new Uint8Array(2 ** 27) } catch (e) { return e.name }").unwrap(), json!("RangeError"));
        });
    }

    // Finding 71: a run dropped mid-await (a trap in a callback) leaves no garbage behind.
    #[test]
    fn dropped_run_frees_its_heap() {
        use futures::FutureExt;
        counted(|| {
            let before = crate::counting_alloc::live();
            let code = format!("const keep = []; for (let i = 0; i < 200000; i++) keep.push({{i}}); await icp.call('{L}', 'm', null); return 1");
            assert!(run(&code, &NeverBridge).now_or_never().is_none());
            let after = crate::counting_alloc::live();
            assert!(after < before + (1 << 20), "{before} -> {after}");
            assert_eq!(ACTIVE_RUNS.with(Cell::get), 0);
        });
    }

    // Finding 99: only the last live run collects the shared heap.
    #[test]
    fn only_the_last_run_collects() {
        #[derive(Trace)]
        struct Garbage(#[unsafe_ignore_trace] Rc<Cell<bool>>);
        impl Finalize for Garbage {
            fn finalize(&self) {
                self.0.set(true);
            }
        }
        let swept = Rc::new(Cell::new(false));
        drop(Gc::new(Garbage(swept.clone())));
        ACTIVE_RUNS.with(|a| a.set(1)); // another run is live
        assert_eq!(js("return 1").unwrap(), json!(1));
        assert!(!swept.get(), "a run collected while another was live");
        assert!(leave());
        assert!(swept.get());
        ACTIVE_RUNS.with(|a| a.set(2));
        assert!(!leave());
        assert!(leave());
        let (a, b) = (YieldBridge(MockBridge::default()), YieldBridge(MockBridge::default()));
        let code = format!("const x = await icp.call('{L}', 'm', null); return x.balance");
        let (ra, rb) = block_on(futures::future::join(run(&code, &a), run("return 1", &b)));
        assert_eq!((ra.unwrap(), rb.unwrap()), (json!(100), json!(1)));
        assert_eq!(ACTIVE_RUNS.with(Cell::get), 0);
    }

    // Finding 111: an array replacer is the spec's property list; excluded keys are never read.
    #[test]
    fn stringify_array_replacer_reads_only_listed_keys() {
        assert_eq!(js("let g = 0; return JSON.stringify({a: 1, get b() { g++; return 1 }}, ['a']) + g").unwrap(), json!("{\"a\":1}0"));
        assert_eq!(js("return JSON.stringify({a: 1, b: 'x'.repeat(8 << 20)}, ['a'])").unwrap(), json!("{\"a\":1}"));
        assert_eq!(js("let d = 1; for (let i = 0; i < 200; i++) d = [d]; return JSON.stringify({a: 1, b: d}, ['a'])").unwrap(), json!("{\"a\":1}"));
        assert_eq!(js("return JSON.stringify({b: 2, a: 1, 1: 3}, ['a', 'b', 1, new String('a'), true])").unwrap(), json!("{\"a\":1,\"b\":2,\"1\":3}"));
        assert_eq!(js("return JSON.stringify({o: {a: 1, z: 2}, a: [{a: 3, z: 4}]}, ['o', 'a'])").unwrap(), json!("{\"o\":{\"a\":1},\"a\":[{\"a\":3}]}"));
        // Listed keys an object lacks cost nothing.
        assert_eq!(js("return JSON.stringify(Array(1000).fill({}), ['k'.repeat(10000)]).length").unwrap(), json!(3001));
        assert_eq!(js("return JSON.stringify(Array(1000).fill({}), Array.from({length: 1000}, (_, i) => 'k' + i)).length").unwrap(), json!(3001));
    }

    // Finding 81: a request queued while converting the result was not sent: an error.
    #[test]
    fn requests_queued_by_result_conversion_are_reported() {
        let b = MockBridge::default();
        let e = block_on(run(&format!("return {{ toJSON() {{ icp.call('{L}', 'icrc1_decimals', null); return 'placed' }} }}"), &b)).unwrap_err();
        assert!(e.contains("never awaited"), "{e}");
        let e = block_on(run(&format!("icp.call('{L}', 'icrc1_decimals', null); throw new Error('x')"), &b)).unwrap_err();
        assert!(e.contains("Error: x") && e.contains("never sent"), "{e}");
        assert!(b.calls.borrow().is_empty());
    }

    // Finding 131: thrown values are formatted within bounds.
    #[test]
    fn thrown_values_are_bounded() {
        let e = js("const e = new AggregateError([]); e.errors.length = 4294967295; throw e").unwrap_err();
        assert!(e.starts_with("AggregateError"), "{e}");
        let e = js("throw new Error('x'.repeat(1 << 24))").unwrap_err();
        assert!(e.len() < MAX_ERROR_BYTES + 64, "{}", e.len());
        let e = js("throw 'y'.repeat(1 << 24)").unwrap_err();
        assert!(e.len() < MAX_ERROR_BYTES + 64, "{}", e.len());
    }

    // Finding 139: extra arguments are refused, as in Python and Rhai, and nothing is sent.
    #[test]
    fn extra_arguments_are_refused() {
        let b = MockBridge::default();
        for code in [
            format!("return await icp.call('{L}', 'icrc1_balance_of', {{owner: 'aaaaa-aa'}}, {{subaccount: [1]}}, 'memo')"),
            format!("return await icp.call('{L}', 'm', undefined, {{}})"),
            format!("return await icp.candid('{L}', 1)"),
            "return icp.principal(1)".to_string(),
        ] {
            let e = block_on(run(&code, &b)).unwrap_err();
            assert!(e.contains("TypeError"), "{code}: {e}");
        }
        assert!(b.calls.borrow().is_empty());
        assert!(js(&format!("return await icp.call('{L}', 'm')")).is_ok());
    }

    // Finding 123: comparators that are not a total order cannot panic the engine.
    #[test]
    fn inconsistent_comparators_do_not_panic() {
        assert_eq!(js("const a = []; for (let i = 0; i < 40; i++) a.push(i % 5 ? {price: i} : {}); return a.sort((x, y) => x.price - y.price).length").unwrap(), json!(40));
        assert_eq!(js("let s = 1; const a = []; for (let i = 0; i < 300; i++) a.push(i); return a.toSorted(() => (s = s * 7919 % 65537) % 3 - 1).length").unwrap(), json!(300));
        assert_eq!(js("return new Int32Array(200).map((_, i) => i).sort((a, b) => (a * 7919) % 3 - 1).length").unwrap(), json!(200));
        // Still a correct, stable sort.
        assert_eq!(js("return [3, 1, 2, 10, undefined, , 1].sort()").unwrap(), json!([1, 1, 10, 2, 3, null, null]));
        assert_eq!(js("return [[1, 'a'], [0, 'b'], [1, 'c'], [0, 'd']].sort((x, y) => x[0] - y[0]).map(x => x[1]).join('')").unwrap(), json!("bdac"));
        assert!(js("return [2, 1].sort(() => { throw new Error('cmp') })").unwrap_err().contains("cmp"));
    }

    // Finding 168: script code run while converting arguments cannot overfill a batch.
    #[test]
    fn pending_cap_holds_against_reentrant_calls() {
        let b = MockBridge::default();
        let code = format!(
            "const ps = []; const arg = {{ get owner() {{ for (let i = 0; i < 64; i++) ps.push(icp.call('{L}', 'm', null)); return 'x' }} }}; \
             ps.push(icp.call('{L}', 'm', arg)); return (await Promise.all(ps)).length"
        );
        let e = block_on(run(&code, &b)).unwrap_err();
        assert!(e.contains("concurrent icp requests"), "{e}");
        assert!(b.calls.borrow().is_empty());
    }

    // Finding 177: console output and uncaught values use the language's own text.
    #[test]
    fn console_and_uncaught_text() {
        let b = MockBridge::default();
        let code = "console.log(NaN, Infinity, -0, Symbol('s'), function f() {}, [1n], null, undefined, {a: 1}); \
                    try { await icp.call('aaaaa-aa', 'x', null) } catch (e) { console.log('failed', e) } return 1";
        block_on(run(code, &b)).unwrap();
        let logs = b.logs.take();
        assert_eq!(logs[0], "NaN Infinity 0 Symbol(s) [Function f] [\"1n\"] null undefined {\"a\":1}");
        assert!(logs[1].starts_with("failed Error: policy"), "{}", logs[1]);
        assert_eq!(js("throw NaN").unwrap_err(), "Uncaught NaN");
        assert_eq!(js("throw undefined").unwrap_err(), "Uncaught undefined");
        assert_eq!(js("throw function g() {}").unwrap_err(), "Uncaught [Function g]");
    }

    // Finding 179: dynamic import() is rejected (catchable), not left pending.
    #[test]
    fn dynamic_import_is_rejected() {
        let r = js("try { await import('x'); return 'loaded' } catch (e) { return 'rejected: ' + e }").unwrap();
        assert!(r.as_str().unwrap().starts_with("rejected:"), "{r}");
        let e = js("await import('x')").unwrap_err();
        assert!(e.contains("module"), "{e}");
    }

    // Finding 84: deep source and deep data end in a clean error, not a native stack overflow.
    #[test]
    fn deep_nesting_is_refused() {
        // Boa's parser uses far more stack per level in a debug build than in release wasm.
        std::thread::Builder::new().stack_size(64 << 20).spawn(deep_nesting_cases).unwrap().join().unwrap();
    }

    fn deep_nesting_cases() {
        let paren = |n: usize| format!("return {}1{}", "(".repeat(n), ")".repeat(n));
        assert_eq!(js(&paren(100)).unwrap(), json!(1));
        assert_eq!(js(&format!("const x = 1; return x{}", "+x".repeat(1000))).unwrap(), json!(1001));
        for code in [paren(500), format!("return {}1", "!".repeat(5000)), format!("return {}1", "- ".repeat(5000)),
                     format!("let a; return {}1", "a = ".repeat(1000)), format!("return {}1", "x => ".repeat(1000)),
                     format!("return {}1{}", "[".repeat(300), "]".repeat(300)), format!("return `${{{}1{}}}`", "`${".repeat(200), "}`".repeat(200)),
                     format!("const x = 1; return x{}", "+x".repeat(5000))] {
            let e = js(&code).unwrap_err();
            assert!(e.contains("nests deeper"), "{}: {e}", &code[..20]);
        }
        // Brackets in strings, comments, regexes and template text do not count.
        let quiet = format!(
            "const s = '{p}', t = `{p}${{1}}{p}`; /* {p} */ // {p}\n if (s) /\\(\\[/.test(s); const r = /[(]{{2}}/; return [s.length, (1) / 2, r.source.length]",
            p = "((([[[{{{".repeat(50)
        );
        assert_eq!(js(&quiet).unwrap(), json!([450, 0.5, 6]));
        assert!(check_nesting(SNAPSHOT_JS).is_ok());
        assert!(check_nesting(&"x = a.b + c * d - e\n".repeat(500)).is_ok());
        assert!(check_nesting(&format!("x{}", "\n= y".repeat(500))).is_err());
        // Statements nest without brackets.
        for code in [format!("{}x", "if (1) ".repeat(2000)), format!("if (0) {{}}{}", " else if (0) {}".repeat(2000)),
                     format!("if (0) x;{}", "\nelse if (0) x; /* */".repeat(2000)), format!("{}x;", "do ".repeat(20000)),
                     format!("{}x", "a: ".repeat(3000)), format!("{}x", "while (0)\n".repeat(2000)),
                     format!("return typeof class extends {}Object{}", "class extends ".repeat(1000), " {}".repeat(1000)),
                     // A keyword as a property name or identifier is not one: `/` after it divides.
                     format!("const o = {{return: 1}}; return o.return / {}1{} / 1", "(".repeat(5000), ")".repeat(5000)),
                     format!("let of = 1; return of / {}1{} / 1", "(".repeat(5000), ")".repeat(5000)),
                     format!("const o = {{if: () => 1}}; return o.if(1) / {}1{} / 1", "(".repeat(5000), ")".repeat(5000))] {
            assert!(check_nesting(&code).unwrap_err().contains("nests deeper"), "{}", &code[..30]);
        }
        // Sequential statements do not.
        for code in ["if (a) return\n", "if (a) {}\n", "if (a) {} ", "if (a) b();", "while (a) b++\n", "for (const x of y) {}",
                     "class A extends B {} ", "switch (a) { case 1: b: c; }\n", "x = {a: 1, b: {c: 2}};", "do x(); while (a)\n"] {
            assert!(check_nesting(&code.repeat(500)).is_ok(), "{code}");
        }
        assert!(check_nesting(&format!("if (0) {{}}{}", " else if (0) {}".repeat(100))).is_ok());
        // Deep data: native recursion stops at the stack floor with a RangeError.
        for code in [
            "let o = {}; for (let i = 0; i < 100000; i++) o = Object.create(o); return o.x",
            "let a = 1; for (let i = 0; i < 20000; i++) a = [a]; return String(a)",
            "let a = 1; for (let i = 0; i < 20000; i++) a = [a]; return a.flat(Infinity)",
            "let a = 1; for (let i = 0; i < 20000; i++) a = [a]; return a < 1",
        ] {
            let e = js(code).unwrap_err();
            assert!(e.contains("native stack"), "{code}: {e}");
        }
        // The floor reached inside steps Boa treats as infallible (`expect`) is an error too:
        // sweep the depth so the getter runs at every distance above it.
        for body in ["return {...src}", "return new Error('m')", "const {a, ...r} = src; return r", "return import('x')",
                     "return Promise.resolve(1)", "return [...[1, 2]]", "return [1, 2].map(x => x)", "return (async () => {})()"] {
            let code = format!("const src = {{a: 1, b: 2}}; let o = {{ get x() {{ {body} }} }}, fails = 0; \
                                for (let d = 0; fails < 300; d++) {{ o = Object.create(o); try {{ o.x }} catch (e) {{ fails++ }} }} return fails");
            // Either caught, or (where Boa turns it into an uncatchable `PanicError`) a clean error.
            match js(&code) {
                Ok(v) => assert_eq!(v, json!(300), "{body}"),
                Err(e) => assert!(e.contains("native stack limit exceeded"), "{body}: {e}"),
            }
        }
        let e = js("let p = {}; for (let i = 0; i < 2000; i++) p = new Proxy(p, {}); return Object.isExtensible(p)").unwrap_err();
        assert!(e.contains("Proxy target chain"), "{e}");
        assert_eq!(js("let p = {x: 1}; for (let i = 0; i < 100; i++) p = new Proxy(p, {}); return p.x").unwrap(), json!(1));
    }
}
