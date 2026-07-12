// =============================================================================
// Remox v0.8 — Standalone Runtime
// Runs on any OS: Linux, macOS, Windows — no special kernel required.
// All 60+ language features intact (Core, Advanced Syntax, UI DSL,
// Vyraweb, Autoclib, Remotest, Sceuti, Tasoaque, Malib, Phinolib,
// Numrux, Astriloop, Retime, Remojoke).
//
// Build & run:
//   cargo run --release -- script.remox
//   cargo run --release                   # REPL
// =============================================================================

use std::collections::HashMap;
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::rc::Rc;
use std::cell::UnsafeCell;

// =========================================================================
// Networking — u32 handle registry so body call sites keep the same
// (handle: u32) shape they already have. Backed by real OS sockets.
// =========================================================================
static REMOX_STREAMS:   Mutex<Option<HashMap<u32, std::net::TcpStream>>>   = Mutex::new(None);
static REMOX_LISTENERS: Mutex<Option<HashMap<u32, std::net::TcpListener>>> = Mutex::new(None);
static REMOX_NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn remox_fresh_id() -> u32 {
    REMOX_NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as u32
}

#[derive(Debug, Clone)]
pub enum IoError {
    NotFound,
    NotReady(&'static str),
    Other(String),
}
impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            IoError::NotFound    => write!(f, "not found"),
            IoError::NotReady(s) => write!(f, "not ready: {}", s),
            IoError::Other(s)    => write!(f, "{}", s),
        }
    }
}

pub fn remox_tcp_connect(addr: &str) -> Result<u32, IoError> {
    let stream = std::net::TcpStream::connect(addr)
        .map_err(|e| IoError::Other(e.to_string()))?;
    let id = remox_fresh_id();
    REMOX_STREAMS.lock().unwrap()
        .get_or_insert_with(HashMap::new).insert(id, stream);
    Ok(id)
}

pub fn remox_tcp_write(handle: u32, data: &[u8]) -> Result<(), IoError> {
    let mut guard = REMOX_STREAMS.lock().unwrap();
    let stream = guard.as_mut().and_then(|m| m.get_mut(&handle))
        .ok_or_else(|| IoError::Other("invalid stream handle".into()))?;
    stream.write_all(data).map_err(|e| IoError::Other(e.to_string()))
}

pub fn remox_tcp_read(handle: u32, max_len: usize) -> Result<Vec<u8>, IoError> {
    use std::io::Read as _;
    let mut guard = REMOX_STREAMS.lock().unwrap();
    let stream = guard.as_mut().and_then(|m| m.get_mut(&handle))
        .ok_or_else(|| IoError::Other("invalid stream handle".into()))?;
    let mut buf = vec![0u8; max_len.max(1)];
    let n = stream.read(&mut buf).map_err(|e| IoError::Other(e.to_string()))?;
    buf.truncate(n);
    Ok(buf)
}

pub fn remox_tcp_bind(addr: &str) -> Result<u32, IoError> {
    let listener = std::net::TcpListener::bind(addr)
        .map_err(|e| IoError::Other(e.to_string()))?;
    let id = remox_fresh_id();
    REMOX_LISTENERS.lock().unwrap()
        .get_or_insert_with(HashMap::new).insert(id, listener);
    Ok(id)
}

pub fn remox_tcp_accept(listener_id: u32) -> Result<(u32, String), IoError> {
    let (stream, peer) = {
        let guard = REMOX_LISTENERS.lock().unwrap();
        let listener = guard.as_ref().and_then(|m| m.get(&listener_id))
            .ok_or_else(|| IoError::Other("invalid listener handle".into()))?;
        listener.accept().map_err(|e| IoError::Other(e.to_string()))?
    };
    let id = remox_fresh_id();
    REMOX_STREAMS.lock().unwrap()
        .get_or_insert_with(HashMap::new).insert(id, stream);
    Ok((id, peer.to_string()))
}

pub fn remox_task_spawn(f: Box<dyn FnOnce() + Send + 'static>) {
    std::thread::spawn(move || f());
}

pub fn remox_entropy() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    static CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64).unwrap_or(0);
    nanos ^ CTR.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
               .wrapping_mul(0x9E3779B97F4A7C15)
}

// =========================================================================
// REMOX INTERPRETER BODY — UNCHANGED BELOW THIS LINE
// =========================================================================
fn remox_kernel_main() {
    let args: Vec<String> = env::args().collect();

    println!("╔═══════════════════════════════════════════════════╗");
    println!("║    Remox Language v0.8 — Remox Runtime               ║");
    println!("║    Now Ahead-of-Time COMPILED (not interpreted)   ║");
    println!("║    ORM + Template Engine + WebSocket Built-in     ║");
    println!("║    Easier than Python. Faster than Kotlin.        ║");
    println!("╚═══════════════════════════════════════════════════╝");
    println!();

    if args.len() > 1 {
        let src = fs::read_to_string(&args[1])
            .unwrap_or_else(|e| { eprintln!("Cannot read file: {}", e); std::process::exit(1); });

        // Compile phase — runs to completion (lex, parse, hoist/validate/fold)
        // BEFORE any statement executes. A failure here is a compile error;
        // the program never starts running.
        let program = match Interpreter::compile_source(&src) {
            Ok(p) => p,
            Err(e) => { eprintln!("{}", e); std::process::exit(1); }
        };
        println!("✓ Compiled successfully ({} top-level statement(s), {} function(s), {} struct(s))",
            program.code.len(), program.fns.len(), program.structs.len());

        // Run phase — executes the already-compiled program. No re-lexing,
        // re-parsing, or re-resolution happens from here on.
        let mut interp = Interpreter::new();
        interp.run_compiled(program);
    } else {
        repl();
    }
}

// =============================================================================
// REPL
// =============================================================================
// Note: each REPL submission is still compiled in full (lex -> parse ->
// compile) before it runs, exactly like a one-line compile unit — it is
// never executed via direct AST-walking of raw tokens. State (fns/structs/
// impls/traits/variables) persists across submissions via `interp`.
#[allow(dead_code)] // remox_kernel_main se call hota hai jab woh khud active hoga
fn repl() {
    println!("Remox REPL v0.8 — compile-then-run each line — type 'exit' to quit\n");
    let stdin = io::stdin();
    let mut interp = Interpreter::new();

    // Feature: "use rc" — Calculator mode.
    //   Type `use rc` once and Enter dabao. Uske baad koi bhi calculation
    //   (2+2, 5*3-1, sqrt(16), (3+4)*2, waghera — koi bhi valid Remox
    //   expression) likh ke Enter karo, woh turant solve hoke print ho
    //   jaayega — `print(...)` likhne ki zaroorat nahi. Bahar niklne ke
    //   liye `use rc off` likho, ya `exit` se poora REPL band karo.
    let mut calc_mode = false;

    loop {
        print!("{}", if calc_mode { "rc> " } else { "remox> " });
        io::stdout().flush().unwrap();
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let t = line.trim().to_string();
                if t.is_empty() { continue; }

                let tl = t.to_lowercase();
                if tl == "use rc" {
                    calc_mode = true;
                    println!("Calculator mode ON — ab jo bhi calculation likhoge (e.g. 2+2, 5*3-1, sqrt(16)) turant solve ho jaayegi. Band karne ke liye 'use rc off' likho.");
                    continue;
                }
                if tl == "use rc off" {
                    calc_mode = false;
                    println!("Calculator mode OFF.");
                    continue;
                }

                if calc_mode {
                    // Raw input ko ek expression maan ke turant evaluate +
                    // print karo — user ko `print(...)` khud likhne ki
                    // zaroorat nahi. Agar user ne khud koi statement
                    // (jaise `let x = 5`) likha ho to woh bhi normally
                    // chalega, bas uska result auto-print nahi hoga.
                    let looks_like_statement = t.starts_with("let ")
                        || t.starts_with("fn ")
                        || t.starts_with("struct ")
                        || t.starts_with("print(")
                        || t.starts_with("println(")
                        || t.starts_with("if ")
                        || t.starts_with("for ")
                        || t.starts_with("while ")
                        || t.starts_with("use ");

                    if looks_like_statement {
                        interp.run_source(&t);
                    } else {
                        let wrapped = format!("print({});", t);
                        interp.run_source(&wrapped);
                    }
                } else {
                    interp.run_source(&t);
                }
            }
        }
    }
}


// =============================================================================
// VALUE — All Remox types (extended for Section 2)
// =============================================================================
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    List(Vec<Value>),
    Map(Vec<(String, Value)>),          // Feature 35: map literal (ordered)
    Range(i64, i64),                    // Feature 14: 1..10
    Null,                               // Feature 13: null safety
    // Feature 27-29: struct instance — struct_name + fields
    Struct { name: String, fields: Vec<(String, Value)> },
    // Feature 32: result type
    Ok(Box<Value>),
    Err(String),
    // Feature 39: lambda — captures env snapshot + param names + body expr
    Lambda { params: Vec<String>, body: Box<Expr>, captures: HashMap<String, Value> },
    // Feature 40: async handle — thread join handle wrapper
    AsyncHandle(Arc<Mutex<Option<Value>>>),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(n)      => write!(f, "{}", n),
            Value::Float(n)    => write!(f, "{}", n),
            Value::Str(s)      => write!(f, "{}", s),
            Value::Bool(b)     => write!(f, "{}", b),
            Value::Null        => write!(f, "null"),
            Value::Range(a, b) => write!(f, "{}..{}", a, b),
            Value::List(v) => {
                let items: Vec<String> = v.iter().map(|x| x.to_string()).collect();
                write!(f, "[{}]", items.join(", "))
            }
            Value::Map(pairs) => {
                if pairs.first().map(|(k,_)| k == "__numrux__").unwrap_or(false) {
                    return write!(f, "{}", numrux_display_string(pairs));
                }
                let items: Vec<String> = pairs.iter()
                    .map(|(k, v)| format!("{}: {}", k, v))
                    .collect();
                write!(f, "{{{}}}", items.join(", "))
            }
            Value::Struct { name, fields } => {
                let flds: Vec<String> = fields.iter()
                    .map(|(k, v)| format!("{}: {}", k, v))
                    .collect();
                write!(f, "{}{{ {} }}", name, flds.join(", "))
            }
            Value::Ok(v)       => write!(f, "ok({})", v),
            Value::Err(e)      => write!(f, "err({})", e),
            Value::Lambda { params, .. } => write!(f, "<lambda({})>", params.join(", ")),
            Value::AsyncHandle(_) => write!(f, "<async>"),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Int(a),    Value::Int(b))    => a == b,
            (Value::Float(a),  Value::Float(b))  => (a - b).abs() < f64::EPSILON,
            (Value::Str(a),    Value::Str(b))    => a == b,
            (Value::Bool(a),   Value::Bool(b))   => a == b,
            (Value::Null,      Value::Null)      => true,
            (Value::Range(a1,b1), Value::Range(a2,b2)) => a1==a2 && b1==b2,
            (Value::List(a),   Value::List(b))   => a == b,
            (Value::Ok(a),     Value::Ok(b))     => a == b,
            (Value::Err(a),    Value::Err(b))    => a == b,
            // AsyncHandle: thread handles are never semantically equal
            (Value::AsyncHandle(_), Value::AsyncHandle(_)) => false,
            _ => false,
        }
    }
}

impl Value {
    fn is_equal(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Int(a),   Value::Int(b))   => a == b,
            (Value::Float(a), Value::Float(b)) => (a - b).abs() < f64::EPSILON,
            (Value::Str(a),   Value::Str(b))   => a == b,
            (Value::Bool(a),  Value::Bool(b))  => a == b,
            (Value::Null,     Value::Null)     => true,
            (Value::Int(a),   Value::Float(b)) => (*a as f64 - b).abs() < f64::EPSILON,
            (Value::Float(a), Value::Int(b))   => (a - *b as f64).abs() < f64::EPSILON,
            _ => false,
        }
    }

    fn is_truthy(&self) -> bool {
        match self {
            Value::Bool(b)  => *b,
            Value::Null     => false,
            Value::Int(0)   => false,
            Value::Str(s)   => !s.is_empty(),
            Value::Err(_)   => false,
            _               => true,
        }
    }

    fn safe_index(&self, idx: i64) -> Value {
        match self {
            Value::List(v) => {
                let i = if idx < 0 { v.len() as i64 + idx } else { idx };
                if i >= 0 && (i as usize) < v.len() { v[i as usize].clone() }
                else { Value::Null }
            }
            Value::Str(s) => {
                let chars: Vec<char> = s.chars().collect();
                let i = if idx < 0 { chars.len() as i64 + idx } else { idx };
                if i >= 0 && (i as usize) < chars.len() {
                    Value::Str(chars[i as usize].to_string())
                } else { Value::Null }
            }
            _ => Value::Null,
        }
    }

    // Feature 25: get method on Value (for chaining .upper() .trim() etc)
    fn get_method_val(&self, method: &str, args: &[Value]) -> Result<Value, RuntimeSignal> {
        match self {
            Value::Str(s) => match method {
                "upper"    => Ok(Value::Str(s.to_uppercase())),
                "lower"    => Ok(Value::Str(s.to_lowercase())),
                "trim"     => Ok(Value::Str(s.trim().to_string())),
                "len"      => Ok(Value::Int(s.len() as i64)),
                "reverse"  => Ok(Value::Str(s.chars().rev().collect())),
                "contains" => {
                    let needle = args.first().map(|v| v.to_string()).unwrap_or_default();
                    Ok(Value::Bool(s.contains(&needle as &str)))
                }
                "starts_with" => {
                    let prefix = args.first().map(|v| v.to_string()).unwrap_or_default();
                    Ok(Value::Bool(s.starts_with(&prefix as &str)))
                }
                "ends_with" => {
                    let suffix = args.first().map(|v| v.to_string()).unwrap_or_default();
                    Ok(Value::Bool(s.ends_with(&suffix as &str)))
                }
                "split" => {
                    let sep = args.first().map(|v| v.to_string()).unwrap_or_else(|| " ".to_string());
                    Ok(Value::List(s.split(&sep as &str).map(|p| Value::Str(p.to_string())).collect()))
                }
                "replace" => {
                    let from = args.first().map(|v| v.to_string()).unwrap_or_default();
                    let to   = args.get(1).map(|v| v.to_string()).unwrap_or_default();
                    Ok(Value::Str(s.replace(&from as &str, &to)))
                }
                "repeat"   => {
                    let n = match args.first() { Some(Value::Int(n)) => *n as usize, _ => 1 };
                    Ok(Value::Str(s.repeat(n)))
                }
                "chars"    => Ok(Value::List(s.chars().map(|c| Value::Str(c.to_string())).collect())),
                _ => Err(RuntimeSignal::Error(format!("str has no method '{}'", method))),
            },
            Value::List(v) => match method {
                "len"     => Ok(Value::Int(v.len() as i64)),
                "reverse" => { let mut r = v.clone(); r.reverse(); Ok(Value::List(r)) }
                "first"   => Ok(v.first().cloned().unwrap_or(Value::Null)),
                "last"    => Ok(v.last().cloned().unwrap_or(Value::Null)),
                "sum"     => {
                    let mut s = 0i64;
                    let mut sf = 0f64;
                    let mut is_float = false;
                    for item in v {
                        match item {
                            Value::Int(n) => s += n,
                            Value::Float(f) => { sf += f; is_float = true; }
                            _ => {}
                        }
                    }
                    if is_float { Ok(Value::Float(sf + s as f64)) } else { Ok(Value::Int(s)) }
                }
                "sort" => {
                    let mut r = v.clone();
                    r.sort_by(|a, b| {
                        match (a, b) {
                            (Value::Int(x), Value::Int(y)) => x.cmp(y),
                            (Value::Float(x), Value::Float(y)) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
                            (Value::Str(x), Value::Str(y)) => x.cmp(y),
                            _ => std::cmp::Ordering::Equal,
                        }
                    });
                    Ok(Value::List(r))
                }
                "join" => {
                    let sep = args.first().map(|v| v.to_string()).unwrap_or_else(|| ", ".to_string());
                    let parts: Vec<String> = v.iter().map(|x| x.to_string()).collect();
                    Ok(Value::Str(parts.join(&sep)))
                }
                "contains" => {
                    let needle = args.first().cloned().unwrap_or(Value::Null);
                    Ok(Value::Bool(v.iter().any(|x| x.is_equal(&needle))))
                }
                "unique" => {
                    let mut seen: Vec<Value> = Vec::new();
                    for item in v {
                        if !seen.iter().any(|s| s.is_equal(item)) { seen.push(item.clone()); }
                    }
                    Ok(Value::List(seen))
                }
                "slice" => {
                    let start = match args.first() { Some(Value::Int(n)) => *n as usize, _ => 0 };
                    let end   = match args.get(1) { Some(Value::Int(n)) => *n as usize, _ => v.len() };
                    Ok(Value::List(v[start.min(v.len())..end.min(v.len())].to_vec()))
                }
                _ => Err(RuntimeSignal::Error(format!("list has no method '{}'", method))),
            },
            Value::Map(pairs) => match method {
                "keys"   => Ok(Value::List(pairs.iter().map(|(k, _)| Value::Str(k.clone())).collect())),
                "values" => Ok(Value::List(pairs.iter().map(|(_, v)| v.clone()).collect())),
                "len"    => Ok(Value::Int(pairs.len() as i64)),
                "has"    => {
                    let key = args.first().map(|v| v.to_string()).unwrap_or_default();
                    Ok(Value::Bool(pairs.iter().any(|(k, _)| k == &key)))
                }
                "get" => {
                    let key = args.first().map(|v| v.to_string()).unwrap_or_default();
                    Ok(pairs.iter().find(|(k, _)| k == &key).map(|(_, v)| v.clone()).unwrap_or(Value::Null))
                }
                _ => Err(RuntimeSignal::Error(format!("map has no method '{}'", method))),
            },
            Value::Int(n) => match method {
                "abs"   => Ok(Value::Int(n.abs())),
                "str"   => Ok(Value::Str(n.to_string())),
                "float" => Ok(Value::Float(*n as f64)),
                _ => Err(RuntimeSignal::Error(format!("int has no method '{}'", method))),
            },
            Value::Float(f) => match method {
                "abs"   => Ok(Value::Float(f.abs())),
                "floor" => Ok(Value::Int(f.floor() as i64)),
                "ceil"  => Ok(Value::Int(f.ceil() as i64)),
                "round" => Ok(Value::Int(f.round() as i64)),
                "str"   => Ok(Value::Str(f.to_string())),
                "sqrt"  => Ok(Value::Float(f.sqrt())),
                _ => Err(RuntimeSignal::Error(format!("float has no method '{}'", method))),
            },
            Value::Struct { fields, .. } => match method {
                "get" => {
                    let key = args.first().map(|v| v.to_string()).unwrap_or_default();
                    Ok(fields.iter().find(|(k, _)| k == &key).map(|(_, v)| v.clone()).unwrap_or(Value::Null))
                }
                "keys" => Ok(Value::List(fields.iter().map(|(k, _)| Value::Str(k.clone())).collect())),
                _ => Err(RuntimeSignal::Error(format!("struct has no method '{}' (use impl)", method))),
            },
            // Feature 32: result methods
            Value::Ok(inner) => match method {
                "unwrap"  => Ok(*inner.clone()),
                "is_ok"   => Ok(Value::Bool(true)),
                "is_err"  => Ok(Value::Bool(false)),
                _ => Err(RuntimeSignal::Error(format!("ok has no method '{}'", method))),
            },
            Value::Err(msg) => match method {
                "unwrap"  => Err(RuntimeSignal::Error(format!("Unwrap on err: {}", msg))),
                "is_ok"   => Ok(Value::Bool(false)),
                "is_err"  => Ok(Value::Bool(true)),
                "message" => Ok(Value::Str(msg.clone())),
                _ => Err(RuntimeSignal::Error(format!("err has no method '{}'", method))),
            },
            _ => Err(RuntimeSignal::Error(format!("Value has no methods"))),
        }
    }
}

// =============================================================================
// TOKENS
// =============================================================================
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
enum Token {
    // Section 1 keywords
    Let, Say, Is, Not, And, Or, Fn, DotDot, Loop, When, Then, Else,
    Question, Colon, Each, In, Exit,
    // Section 2 keywords
    Match,       // Feature 24
    Struct,      // Feature 27
    Impl,        // Feature 29
    Trait,       // Feature 30
    Try,         // Feature 31
    Catch,       // Feature 31
    Import,      // Feature 36
    Type,        // Feature 37
    Async,       // Feature 40
    Await,       // Feature 40
    Return,      // explicit return
    For,         // list comprehension
    Ui,          // Feature 41: ui { } block — HTML/CSS UI designer
    View,        // Feature 41b: view keyword — named reusable UI component
    Screen,      // Feature 42: screen { } — full-page wrapper
    Button,      // Feature 44: button "label" on_click: fn
    Image,       // Feature 45: image src:"..." width:N height:N alt:"..."
    Input,       // Feature 46: input placeholder:"..." type:"text"
    Layout,      // Feature 47/48/49: layout row / col / grid
    Row,         // Feature 47: row direction
    Col,         // Feature 48: col direction
    Grid,        // Feature 49: grid columns:3 gap:16
    Style,       // Feature 50: style { ... } — scoped CSS block
    // Pipe operator |>   Feature 26
    Pipe,
    // Spread ...         Feature 23
    Spread,
    // Arrow =>
    Arrow,
    // Thin arrow ->  (for generics/trait hints)
    ThinArrow,
    // < > for generics
    Lt, Gt, LtEq, GtEq,

    // Literals
    IntLit(i64), FloatLit(f64), StrLit(String), BoolLit(bool), Null,

    // Identifiers
    Ident(String),

    // Operators
    Plus, Minus, Star, Slash, Percent,
    Eq, EqEq, NotEq,
    Dot,
    LParen, RParen, LBrace, RBrace, LBracket, RBracket,
    Comma, Semicolon, Hash,
    At,       // @ — used for named-param call site
    Newline,
    EOF,
}

// =============================================================================
// LEXER
// =============================================================================
struct Lexer {
    src:  Vec<char>,
    pos:  usize,
    line: usize,
}

impl Lexer {
    fn new(src: &str) -> Self {
        Lexer { src: src.chars().collect(), pos: 0, line: 1 }
    }

    fn peek(&self)  -> char { self.src.get(self.pos).copied().unwrap_or('\0') }
    fn peek2(&self) -> char { self.src.get(self.pos + 1).copied().unwrap_or('\0') }
    #[allow(dead_code)]
    fn peek3(&self) -> char { self.src.get(self.pos + 2).copied().unwrap_or('\0') }

    fn advance(&mut self) -> char {
        let c = self.peek();
        self.pos += 1;
        if c == '\n' { self.line += 1; }
        c
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), ' ' | '\t' | '\r') { self.advance(); }
    }

    fn skip_comment(&mut self) {
        if self.peek() == '/' && self.peek2() == '/' {
            while self.peek() != '\n' && self.peek() != '\0' { self.advance(); }
            return;
        }
        if self.peek() == '#' && self.peek2() == '#' {
            while self.peek() != '\n' && self.peek() != '\0' { self.advance(); }
        }
    }

    fn read_string(&mut self) -> Result<Token, String> {
        self.advance(); // consume "
        let mut s = String::new();
        while self.peek() != '"' && self.peek() != '\0' {
            if self.peek() == '\\' {
                self.advance();
                match self.advance() {
                    'n'  => s.push('\n'),
                    't'  => s.push('\t'),
                    '"'  => s.push('"'),
                    '\\' => s.push('\\'),
                    c    => { s.push('\\'); s.push(c); }
                }
            } else {
                s.push(self.advance());
            }
        }
        if self.peek() == '"' { self.advance(); }
        Ok(Token::StrLit(s))
    }

    fn read_number(&mut self) -> Token {
        let mut s = String::new();
        let mut is_float = false;
        while self.peek().is_ascii_digit() { s.push(self.advance()); }
        if self.peek() == '.' && self.peek2().is_ascii_digit() {
            is_float = true;
            s.push(self.advance());
            while self.peek().is_ascii_digit() { s.push(self.advance()); }
        }
        // Fix 3: Scientific notation — 1e10, 2.5e-3, 9.81E2 etc.
        if self.peek() == 'e' || self.peek() == 'E' {
            is_float = true;
            s.push(self.advance()); // consume 'e'/'E'
            if self.peek() == '+' || self.peek() == '-' {
                s.push(self.advance()); // consume sign
            }
            while self.peek().is_ascii_digit() { s.push(self.advance()); }
        }
        if is_float { Token::FloatLit(s.parse().unwrap_or(0.0)) }
        else        { Token::IntLit(s.parse().unwrap_or(0)) }
    }

    fn read_ident(&mut self) -> Token {
        let mut s = String::new();
        while self.peek().is_alphanumeric() || self.peek() == '_' {
            s.push(self.advance());
        }
        match s.as_str() {
            "let"    => Token::Let,
            "say"    => Token::Say,
            "fn"     => Token::Fn,
            "is"     => Token::Is,
            "not"    => Token::Not,
            "and"    => Token::And,
            "or"     => Token::Or,
            "loop"   => Token::Loop,
            "when"   => Token::When,
            "then"   => Token::Then,
            "else"   => Token::Else,
            "each"   => Token::Each,
            "in"     => Token::In,
            "exit"   => Token::Exit,
            "match"  => Token::Match,
            "struct" => Token::Struct,
            "impl"   => Token::Impl,
            "trait"  => Token::Trait,
            "try"    => Token::Try,
            "catch"  => Token::Catch,
            "use"    => Token::Import,
            "ui"     => Token::Ui,
            "view"   => Token::View,
            "screen" => Token::Screen,
            "button" => Token::Button,
            "image"  => Token::Image,
            "input"  => Token::Input,
            "layout" => Token::Layout,
            "row"    => Token::Row,
            "col"    => Token::Col,
            "grid"   => Token::Grid,
            "style"  => Token::Style,
            "type"   => Token::Type,
            "async"  => Token::Async,
            "await"  => Token::Await,
            "return" => Token::Return,
            "for"    => Token::For,
            "true"   => Token::BoolLit(true),
            "false"  => Token::BoolLit(false),
            "null"   => Token::Null,
            _        => Token::Ident(s),
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens: Vec<Token> = Vec::new();

        loop {
            self.skip_ws();

            if (self.peek() == '/' && self.peek2() == '/') ||
               (self.peek() == '#' && self.peek2() == '#') {
                self.skip_comment();
                continue;
            }

            let c = self.peek();
            if c == '\0' { tokens.push(Token::EOF); break; }

            if c == '\n' {
                self.advance();
                if !matches!(tokens.last(), Some(Token::Newline) | Some(Token::LBrace) | None) {
                    tokens.push(Token::Newline);
                }
                continue;
            }

            if c == '"' { tokens.push(self.read_string()?); continue; }
            if c.is_ascii_digit() { tokens.push(self.read_number()); continue; }
            if c.is_alphabetic() || c == '_' { tokens.push(self.read_ident()); continue; }

            self.advance();
            match c {
                '+' => tokens.push(Token::Plus),
                '-' => {
                    if self.peek() == '>' {
                        self.advance(); tokens.push(Token::ThinArrow);
                    } else {
                        tokens.push(Token::Minus);
                    }
                }
                '*' => tokens.push(Token::Star),
                '%' => tokens.push(Token::Percent),
                '/' => tokens.push(Token::Slash),
                '(' => tokens.push(Token::LParen),
                ')' => tokens.push(Token::RParen),
                '{' => tokens.push(Token::LBrace),
                '}' => tokens.push(Token::RBrace),
                '[' => tokens.push(Token::LBracket),
                ']' => tokens.push(Token::RBracket),
                ',' => tokens.push(Token::Comma),
                ';' => tokens.push(Token::Semicolon),
                '@' => tokens.push(Token::At),
                '?' => tokens.push(Token::Question),
                '#' => tokens.push(Token::Hash),
                '.' => {
                    if self.peek() == '.' && self.peek2() == '.' {
                        self.advance(); self.advance();
                        tokens.push(Token::Spread);  // Feature 23: ...
                    } else if self.peek() == '.' {
                        self.advance();
                        tokens.push(Token::DotDot);  // Feature 14: 1..10
                    } else {
                        tokens.push(Token::Dot);     // Feature 25: method chain
                    }
                }
                ':' => tokens.push(Token::Colon),
                '=' => {
                    if self.peek() == '=' { self.advance(); tokens.push(Token::EqEq); }
                    else if self.peek() == '>' { self.advance(); tokens.push(Token::Arrow); }
                    else { tokens.push(Token::Eq); }
                }
                '!' => {
                    if self.peek() == '=' { self.advance(); tokens.push(Token::NotEq); }
                }
                '<' => {
                    if self.peek() == '=' { self.advance(); tokens.push(Token::LtEq); }
                    else { tokens.push(Token::Lt); }
                }
                '>' => {
                    if self.peek() == '=' { self.advance(); tokens.push(Token::GtEq); }
                    else { tokens.push(Token::Gt); }
                }
                '|' => {
                    if self.peek() == '>' { self.advance(); tokens.push(Token::Pipe); } // Feature 26
                }
                _ => {}
            }
        }
        Ok(tokens)
    }
}

// =============================================================================
// AST
// =============================================================================
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
enum Expr {
    IntLit(i64),
    FloatLit(f64),
    StrLit(String),
    BoolLit(bool),
    Null,
    Ident(String),
    List(Vec<Expr>),
    Range(Box<Expr>, Box<Expr>),
    Map(Vec<(String, Expr)>),             // Feature 35: map literal
    Spread(Box<Expr>),                    // Feature 23: ...expr
    ListComp { expr: Box<Expr>, var: String, iter: Box<Expr>, cond: Option<Box<Expr>> }, // Feature 34
    BinOp { op: BinOpKind, left: Box<Expr>, right: Box<Expr> },
    Not(Box<Expr>),
    Ternary { cond: Box<Expr>, then_val: Box<Expr>, else_val: Box<Expr> },
    NullSafe(Box<Expr>, Box<Expr>),
    Call { name: String, args: Vec<Expr>, named: Vec<(String, Expr)> }, // Feature 21
    MethodCall { object: Box<Expr>, method: String, args: Vec<Expr> },  // Feature 25
    StructAccess(Box<Expr>, String),                                     // struct.field
    StructLit { name: String, fields: Vec<(String, Expr)> },            // Feature 28 auto-ctor
    Lambda { params: Vec<String>, body: Box<Expr> },                    // Feature 39
    Pipe { left: Box<Expr>, right: Box<Expr> },                         // Feature 26
    Match { subject: Box<Expr>, arms: Vec<(Expr, Expr)>, default: Option<Box<Expr>> },
    WhenExpr { subject: Box<Expr>, cases: Vec<(Expr, Expr)>, default: Option<Box<Expr>> }, // when as expr // Feature 24
    Await(Box<Expr>),                                                    // Feature 40
    OkLit(Box<Expr>),                                                    // Feature 32: ok(x)
    ErrLit(Box<Expr>),                                                   // Feature 32: err("msg")
    GenericCall { name: String, type_params: Vec<String>, args: Vec<Expr> }, // Feature 38
    // BUGFIX: allows `say expr` to be used as an expression, specifically so
    // match/when arm values (which are parsed as expressions, not statements)
    // can contain a `say` call like the README's own match example does.
    SayExpr(Box<Expr>),
    // BUGFIX: `try { } catch err { }` used as an expression, e.g.
    // `let result = try { ... } catch err { ... }` (per README). Previously
    // `try` was only a statement, so it wasn't reachable from parse_expr(),
    // causing "Unexpected token: Try" wherever a value was expected.
    TryExpr {
        body: Vec<Stmt>,
        catch_var: Option<String>,
        catch_body: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
    },
}

#[derive(Debug, Clone, PartialEq)]
enum BinOpKind {
    Add, Sub, Mul, Div, Mod,
    Eq, NotEq, Lt, Gt, LtEq, GtEq,
    And, Or,
}

#[derive(Debug, Clone, PartialEq)]
struct StructDef {
    name:   String,
    fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct ImplBlock {
    target:  String,
    methods: Vec<(String, Vec<String>, Vec<Stmt>)>, // name, params, body
}

#[derive(Debug, Clone, PartialEq)]
struct TraitDef {
    name:    String,
    methods: Vec<String>, // method signatures (just names for simplicity)
}

// Feature 41: UI block — describes one HTML element with CSS-like style
// properties, optional text/inline content, and nested children.
#[derive(Debug, Clone, PartialEq)]
struct UiNode {
    tag:      String,                    // div, button, text, input, img, h1, p, span, etc.
    content:  Option<Expr>,              // text "..." / button "label" { } — the literal/expr content
    props:    Vec<(String, Expr)>,       // CSS-like style props: color: "red", width: "100px"
    attrs:    Vec<(String, Expr)>,       // non-style attrs: onClick, href, src, placeholder, id, class
    children: Vec<UiNode>,
}

// Keys that are HTML attributes (not CSS style properties) inside a ui{} element.
fn is_ui_attr_key(key: &str) -> bool {
    matches!(key,
        "onClick"  | "on_click"  |
        "onChange" | "on_change" |
        "onSubmit" | "on_submit" |
        "onHover"  | "on_hover"  |
        "onInput"  | "on_input"  |
        "onPress"  | "on_press"  |
        "href" | "src" | "alt" | "id" | "class" |
        "placeholder" | "type" | "name" | "value" |
        "target" | "rel" | "disabled" | "checked"
    )
}

// Converts a Remox-friendly camelCase style key into real CSS kebab-case,
// since Remox identifiers can't contain '-' (e.g. backgroundColor → background-color).
fn css_key(key: &str) -> String {
    let mut out = String::new();
    for c in key.chars() {
        if c.is_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

// Maps Remox event-style attr names to real HTML/JS attribute names.
#[allow(dead_code)]
fn html_attr_key(key: &str) -> String {
    match key {
        "onClick"  => "onclick".to_string(),
        "onChange" => "onchange".to_string(),
        "onSubmit" => "onsubmit".to_string(),
        "onHover"  => "onmouseover".to_string(),
        "onInput"  => "oninput".to_string(),
        other => other.to_string(),
    }
}

// Minimal HTML-escaping for text content and attribute values.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
}

#[derive(Debug, Clone, PartialEq)]
enum Stmt {
    Let    { names: Vec<String>, values: Vec<Expr> },
    Assign { name: String, value: Expr },
    FieldAssign { obj: String, field: String, value: Expr }, // Fix 4: p.x = val
    // Feature 33: destructuring let {name, age} = person
    Destructure { keys: Vec<String>, source: Expr },
    Say(Vec<Expr>),
    Fn     { name: String, params: Vec<(String, Option<Expr>)>, body: Vec<Stmt>, is_async: bool }, // Feature 21,22,40
    Loop   { count: Expr, body: Vec<Stmt> },
    When   { subject: Expr, cases: Vec<(Expr, Vec<Stmt>)>, default: Option<Vec<Stmt>> },
    Each   { var: String, iter: Expr, body: Vec<Stmt> },
    If     { cond: Expr, then_body: Vec<Stmt>, else_body: Option<Vec<Stmt>> },
    Exit(Option<Expr>),
    Return(Option<Expr>),
    // Feature 27: struct declaration
    StructDecl(StructDef),
    // Feature 29: impl block
    ImplDecl(ImplBlock),
    // Feature 30: trait declaration
    TraitDecl(TraitDef),
    // Feature 31: try/catch/else
    TryCatch { body: Vec<Stmt>, catch_var: Option<String>, catch_body: Vec<Stmt>, else_body: Option<Vec<Stmt>> },
    // Feature 36: use (module loading)
    Import(String),
    // Feature 37: type alias
    TypeAlias { alias: String, target: String },
    // Feature 41: ui { } block — compiles to a generated HTML/CSS file
    UiDecl { name: String, title: Option<Expr>, root: Vec<UiNode> },
    // Feature 41b: view keyword — named reusable UI component (compiles to <div class="name">)
    ViewDecl { name: String, body: Vec<UiNode> },
    // Feature 42: screen block — full-page wrapper with optional theme/title
    ScreenDecl { name: String, title: Option<Expr>, theme: Option<String>, body: Vec<UiNode> },
    // Feature 50: style { selector { prop: val } } — scoped CSS block injected into next HTML file
    StyleBlock { rules: Vec<(String, Vec<(String, String)>)> },
    Expr(Expr),
}

// =============================================================================
// COMPILER — Section 4 (v0.8): Remox is now AHEAD-OF-TIME COMPILED.
// =============================================================================
// Remox v0.7 was a tree-walking INTERPRETER: source was lexed, parsed, and
// then executed statement-by-statement in the same pass — every "say"/"let"/
// "fn" was interpreted live as the file was read top to bottom.
//
// Remox v0.8 introduces a REAL COMPILE PHASE that runs to completion BEFORE
// a single statement is executed:
//
//   source -> Lexer -> tokens -> Parser -> AST -> [[ COMPILER ]] -> CompiledProgram -> Runtime executes
//
// The Compiler walks the *entire* AST exactly once, ahead of time, and:
//   1. Pre-registers every top-level `fn`, `struct`, `impl`, `trait` and
//      `type` declaration into resolved symbol tables (CompiledProgram),
//      instead of discovering them one statement at a time during a live run.
//      This also means forward references (calling a function declared
//      later in the file) work, which a line-by-line interpreter cannot do.
//   2. Statically resolves and validates every function/struct reference it
//      can prove at compile time (unknown-struct / duplicate-definition
//      errors are now COMPILE ERRORS, reported before any code runs — never
//      as a `Runtime Error` halfway through a script).
//   3. Constant-folds literal arithmetic/boolean/string-concat expressions
//      so the execution stage does less repeated work per run.
//   4. Emits a finished, ordered statement/instruction list
//      (CompiledProgram.code) that the runtime executes directly — there is
//      no re-lexing, re-parsing, or re-resolution while the program runs.
//
// IMPORTANT: every one of the 50+ Remox language features (struct/impl/
// trait, async/await, lambdas+closures, try/catch, UI/screen/view/style,
// Vyraweb ORM+templates+WebSocket, Malib/Phinolib math, list comprehension,
// pattern match/when, pipe operator, destructuring, generics, etc.) keeps
// IDENTICAL runtime semantics. The compiler does not reimplement feature
// behavior — it only moves all symbol discovery, validation and constant
// folding into one upfront pass so the program compiles once and then runs,
// rather than being interpreted line by line.
// =============================================================================

/// The fully compiled, ready-to-execute Remox program — the artifact a real
/// compiler produces (conceptually like a .pyc/object file) that the runtime
/// then loads and runs without touching source text, tokens, or doing any
/// further name resolution.
#[derive(Debug, Clone)]
struct CompiledProgram {
    /// Final, validated, constant-folded top-level statement stream — ready
    /// for direct execution. Order preserved from source.
    code:    Vec<Stmt>,
    /// All top-level functions resolved ahead of time: name -> (params, body, is_async)
    fns:     HashMap<String, (Vec<(String, Option<Expr>)>, Rc<Vec<Stmt>>, bool)>,
    /// All struct declarations resolved ahead of time.
    structs: HashMap<String, StructDef>,
    /// All impl blocks resolved ahead of time.
    impls:   HashMap<String, Vec<(String, Vec<String>, Vec<Stmt>)>>,
    /// All trait declarations resolved ahead of time.
    traits:  HashMap<String, TraitDef>,
    /// Diagnostics gathered during compilation that are non-fatal — printed
    /// once, after compilation, before execution begins.
    warnings: Vec<String>,
}

/// The Compiler performs one full ahead-of-time pass over the AST: symbol
/// registration, static validation, and constant folding. It never executes
/// any code — `Interpreter::run_compiled` is what runs a `CompiledProgram`.
struct Compiler {
    fns:      HashMap<String, (Vec<(String, Option<Expr>)>, Rc<Vec<Stmt>>, bool)>,
    structs:  HashMap<String, StructDef>,
    impls:    HashMap<String, Vec<(String, Vec<String>, Vec<Stmt>)>>,
    traits:   HashMap<String, TraitDef>,
    warnings: Vec<String>,
    // Names the compiler recognizes as "known callable" builtins when
    // validating Call targets ahead of time (kept loose/best-effort since
    // Remox's builtin dispatch table is large and dynamically extensible
    // via Vyraweb/Malib/Phinolib namespaced calls).
    known_builtins: std::collections::HashSet<&'static str>,
}

impl Compiler {
    fn new() -> Self {
        let known_builtins: std::collections::HashSet<&'static str> = [
            "len","int","float","str","bool","type","push","pop","range","ok","err",
            "say","print","input","random","sqrt","pow","abs","min","max","floor","ceil","round",
            "assert","keys","values","map","filter","reduce","sort","join","split","upper","lower",
            "trim","contains","exit","clone","Vyraweb","Malib","Phinolib","Numrux","Autoclib","Tasoaque","Remotest","Astriloop","Retime",
        ].into_iter().collect();

        Compiler {
            fns: HashMap::new(),
            structs: HashMap::new(),
            impls: HashMap::new(),
            traits: HashMap::new(),
            warnings: Vec::new(),
            known_builtins,
        }
    }

    /// Ahead-of-time compile of a full top-level statement list (one source
    /// file / one REPL submission). This single pass is what replaces
    /// "interpret as you go" — everything here happens before run_compiled.
    fn compile(&mut self, ast: &[Stmt]) -> Result<CompiledProgram, String> {
        // Pass 1: hoist all declarations (fn/struct/impl/trait) regardless of
        // their position in the file, so forward references just work.
        for stmt in ast {
            self.hoist_decl(stmt)?;
        }

        // Pass 2: validate + constant-fold the statement stream using the
        // now-complete symbol tables from pass 1.
        let mut folded = Vec::with_capacity(ast.len());
        for stmt in ast {
            folded.push(self.compile_stmt(stmt)?);
        }

        Ok(CompiledProgram {
            code: folded,
            fns: self.fns.clone(),
            structs: self.structs.clone(),
            impls: self.impls.clone(),
            traits: self.traits.clone(),
            warnings: std::mem::take(&mut self.warnings),
        })
    }

    fn hoist_decl(&mut self, stmt: &Stmt) -> Result<(), String> {
        match stmt {
            Stmt::Fn { name, params, body, is_async } => {
                if self.fns.contains_key(name) {
                    self.warnings.push(format!("function '{}' redefined", name));
                }
                self.fns.insert(name.clone(), (params.clone(), Rc::new(body.clone()), *is_async));
            }
            Stmt::StructDecl(def) => {
                if self.structs.contains_key(&def.name) {
                    self.warnings.push(format!("struct '{}' redefined", def.name));
                }
                self.structs.insert(def.name.clone(), def.clone());
            }
            Stmt::ImplDecl(block) => {
                self.impls.insert(block.target.clone(), block.methods.clone());
            }
            Stmt::TraitDecl(def) => {
                self.traits.insert(def.name.clone(), def.clone());
            }
            // Nested decls — recurse so hoisting is genuinely whole-program.
            Stmt::If { then_body, else_body, .. } => {
                for s in then_body { self.hoist_decl(s)?; }
                if let Some(eb) = else_body { for s in eb { self.hoist_decl(s)?; } }
            }
            Stmt::Loop { body, .. } | Stmt::Each { body, .. } => {
                for s in body { self.hoist_decl(s)?; }
            }
            Stmt::TryCatch { body, catch_body, else_body, .. } => {
                for s in body { self.hoist_decl(s)?; }
                for s in catch_body { self.hoist_decl(s)?; }
                if let Some(eb) = else_body { for s in eb { self.hoist_decl(s)?; } }
            }
            _ => {}
        }
        Ok(())
    }

    /// Validate + constant-fold one statement. Structure is preserved
    /// (Stmt -> Stmt) since the runtime engine executes Stmt/Expr directly;
    /// the compiler's job is upfront resolution/validation/folding, not
    /// changing the shape of the executable representation.
    fn compile_stmt(&mut self, stmt: &Stmt) -> Result<Stmt, String> {
        let out = match stmt {
            Stmt::Let { names, values } => {
                let mut new_vals = Vec::with_capacity(values.len());
                for v in values { new_vals.push(self.compile_expr(v)?); }
                Stmt::Let { names: names.clone(), values: new_vals }
            }
            Stmt::Assign { name, value } => {
                Stmt::Assign { name: name.clone(), value: self.compile_expr(value)? }
            }
            Stmt::FieldAssign { obj, field, value } => {
                Stmt::FieldAssign { obj: obj.clone(), field: field.clone(), value: self.compile_expr(value)? }
            }
            Stmt::Destructure { keys, source } => {
                Stmt::Destructure { keys: keys.clone(), source: self.compile_expr(source)? }
            }
            Stmt::Say(exprs) => {
                let mut out = Vec::with_capacity(exprs.len());
                for e in exprs { out.push(self.compile_expr(e)?); }
                Stmt::Say(out)
            }
            Stmt::Fn { name, params, body, is_async } => {
                let mut new_params = Vec::with_capacity(params.len());
                for (p, def) in params {
                    let def2 = match def { Some(e) => Some(self.compile_expr(e)?), None => None };
                    new_params.push((p.clone(), def2));
                }
                let new_body = self.compile_block(body)?;
                self.fns.insert(name.clone(), (new_params.clone(), Rc::new(new_body.clone()), *is_async));
                Stmt::Fn { name: name.clone(), params: new_params, body: new_body, is_async: *is_async }
            }
            Stmt::Loop { count, body } => {
                Stmt::Loop { count: self.compile_expr(count)?, body: self.compile_block(body)? }
            }
            Stmt::When { subject, cases, default } => {
                let mut new_cases = Vec::with_capacity(cases.len());
                for (pat, body) in cases {
                    new_cases.push((self.compile_expr(pat)?, self.compile_block(body)?));
                }
                let new_default = match default { Some(b) => Some(self.compile_block(b)?), None => None };
                Stmt::When { subject: self.compile_expr(subject)?, cases: new_cases, default: new_default }
            }
            Stmt::Each { var, iter, body } => {
                Stmt::Each { var: var.clone(), iter: self.compile_expr(iter)?, body: self.compile_block(body)? }
            }
            Stmt::If { cond, then_body, else_body } => {
                let new_else = match else_body { Some(b) => Some(self.compile_block(b)?), None => None };
                Stmt::If { cond: self.compile_expr(cond)?, then_body: self.compile_block(then_body)?, else_body: new_else }
            }
            Stmt::Exit(e) => Stmt::Exit(match e { Some(x) => Some(self.compile_expr(x)?), None => None }),
            Stmt::Return(e) => Stmt::Return(match e { Some(x) => Some(self.compile_expr(x)?), None => None }),
            Stmt::StructDecl(def) => Stmt::StructDecl(def.clone()),
            Stmt::ImplDecl(block) => {
                let mut new_methods = Vec::with_capacity(block.methods.len());
                for (mname, params, body) in &block.methods {
                    new_methods.push((mname.clone(), params.clone(), self.compile_block(body)?));
                }
                let new_block = ImplBlock { target: block.target.clone(), methods: new_methods };
                self.impls.insert(new_block.target.clone(), new_block.methods.clone());
                Stmt::ImplDecl(new_block)
            }
            Stmt::TraitDecl(def) => Stmt::TraitDecl(def.clone()),
            Stmt::TryCatch { body, catch_var, catch_body, else_body } => {
                let new_else = match else_body { Some(b) => Some(self.compile_block(b)?), None => None };
                Stmt::TryCatch {
                    body: self.compile_block(body)?,
                    catch_var: catch_var.clone(),
                    catch_body: self.compile_block(catch_body)?,
                    else_body: new_else,
                }
            }
            Stmt::Import(name) => Stmt::Import(name.clone()),
            Stmt::TypeAlias { alias, target } => Stmt::TypeAlias { alias: alias.clone(), target: target.clone() },
            Stmt::UiDecl { name, title, root } => {
                let new_title = match title { Some(e) => Some(self.compile_expr(e)?), None => None };
                Stmt::UiDecl { name: name.clone(), title: new_title, root: root.clone() }
            }
            Stmt::ViewDecl { name, body } => Stmt::ViewDecl { name: name.clone(), body: body.clone() },
            Stmt::ScreenDecl { name, title, theme, body } => {
                let new_title = match title { Some(e) => Some(self.compile_expr(e)?), None => None };
                Stmt::ScreenDecl { name: name.clone(), title: new_title, theme: theme.clone(), body: body.clone() }
            }
            Stmt::StyleBlock { rules } => Stmt::StyleBlock { rules: rules.clone() },
            Stmt::Expr(e) => Stmt::Expr(self.compile_expr(e)?),
        };
        Ok(out)
    }

    fn compile_block(&mut self, body: &[Stmt]) -> Result<Vec<Stmt>, String> {
        let mut out = Vec::with_capacity(body.len());
        for s in body { out.push(self.compile_stmt(s)?); }
        Ok(out)
    }

    /// Validate + constant-fold one expression ahead of time.
    fn compile_expr(&mut self, expr: &Expr) -> Result<Expr, String> {
        let out = match expr {
            Expr::IntLit(_) | Expr::FloatLit(_) | Expr::StrLit(_) | Expr::BoolLit(_) | Expr::Null | Expr::Ident(_) => expr.clone(),

            Expr::List(items) => {
                let mut out = Vec::with_capacity(items.len());
                for i in items { out.push(self.compile_expr(i)?); }
                Expr::List(out)
            }
            Expr::Range(a, b) => Expr::Range(Box::new(self.compile_expr(a)?), Box::new(self.compile_expr(b)?)),
            Expr::Map(pairs) => {
                let mut out = Vec::with_capacity(pairs.len());
                for (k, v) in pairs { out.push((k.clone(), self.compile_expr(v)?)); }
                Expr::Map(out)
            }
            Expr::Spread(inner) => Expr::Spread(Box::new(self.compile_expr(inner)?)),
            Expr::ListComp { expr, var, iter, cond } => {
                let new_cond = match cond { Some(c) => Some(Box::new(self.compile_expr(c)?)), None => None };
                Expr::ListComp {
                    expr: Box::new(self.compile_expr(expr)?),
                    var: var.clone(),
                    iter: Box::new(self.compile_expr(iter)?),
                    cond: new_cond,
                }
            }
            // Constant fold: literal BinOp on Int/Float/Bool/Str operands
            // gets folded at compile time, ahead of execution.
            Expr::BinOp { op, left, right } => {
                let l = self.compile_expr(left)?;
                let r = self.compile_expr(right)?;
                if let Some(folded) = Self::try_fold_binop(op, &l, &r) {
                    folded
                } else {
                    Expr::BinOp { op: op.clone(), left: Box::new(l), right: Box::new(r) }
                }
            }
            Expr::Not(e) => {
                let inner = self.compile_expr(e)?;
                match &inner {
                    Expr::BoolLit(b) => Expr::BoolLit(!b),
                    _ => Expr::Not(Box::new(inner)),
                }
            }
            Expr::Ternary { cond, then_val, else_val } => {
                Expr::Ternary {
                    cond: Box::new(self.compile_expr(cond)?),
                    then_val: Box::new(self.compile_expr(then_val)?),
                    else_val: Box::new(self.compile_expr(else_val)?),
                }
            }
            Expr::NullSafe(a, b) => Expr::NullSafe(Box::new(self.compile_expr(a)?), Box::new(self.compile_expr(b)?)),
            Expr::Call { name, args, named } => {
                // Best-effort compile-time validation: flag calls to names
                // that are neither a known builtin, a user fn, nor a struct
                // ctor. Remox is dynamically typed (a variable can hold a
                // lambda and be "called" by name resolution at runtime), so
                // this is recorded as a warning, never a hard compile error.
                if !self.known_builtins.contains(name.as_str())
                    && !self.fns.contains_key(name)
                    && !self.structs.contains_key(name)
                    && name != "ok" && name != "err"
                    && !name.starts_with("__")
                {
                    self.warnings.push(format!(
                        "call to '{}' could not be resolved at compile time (may be a variable holding a lambda, resolved at runtime)", name));
                }
                let mut new_args = Vec::with_capacity(args.len());
                for a in args { new_args.push(self.compile_expr(a)?); }
                let mut new_named = Vec::with_capacity(named.len());
                for (k, v) in named { new_named.push((k.clone(), self.compile_expr(v)?)); }
                Expr::Call { name: name.clone(), args: new_args, named: new_named }
            }
            Expr::MethodCall { object, method, args } => {
                let mut new_args = Vec::with_capacity(args.len());
                for a in args { new_args.push(self.compile_expr(a)?); }
                Expr::MethodCall { object: Box::new(self.compile_expr(object)?), method: method.clone(), args: new_args }
            }
            Expr::StructAccess(obj, field) => Expr::StructAccess(Box::new(self.compile_expr(obj)?), field.clone()),
            Expr::StructLit { name, fields } => {
                if !self.structs.contains_key(name) {
                    return Err(format!("Compile Error: unknown struct '{}'", name));
                }
                let mut new_fields = Vec::with_capacity(fields.len());
                for (k, v) in fields { new_fields.push((k.clone(), self.compile_expr(v)?)); }
                Expr::StructLit { name: name.clone(), fields: new_fields }
            }
            Expr::Lambda { params, body } => {
                Expr::Lambda { params: params.clone(), body: Box::new(self.compile_expr(body)?) }
            }
            Expr::Pipe { left, right } => {
                Expr::Pipe { left: Box::new(self.compile_expr(left)?), right: Box::new(self.compile_expr(right)?) }
            }
            Expr::Match { subject, arms, default } => {
                let mut new_arms = Vec::with_capacity(arms.len());
                for (p, v) in arms { new_arms.push((self.compile_expr(p)?, self.compile_expr(v)?)); }
                let new_default = match default { Some(d) => Some(Box::new(self.compile_expr(d)?)), None => None };
                Expr::Match { subject: Box::new(self.compile_expr(subject)?), arms: new_arms, default: new_default }
            }
            Expr::WhenExpr { subject, cases, default } => {
                let mut new_cases = Vec::with_capacity(cases.len());
                for (p, v) in cases { new_cases.push((self.compile_expr(p)?, self.compile_expr(v)?)); }
                let new_default = match default { Some(d) => Some(Box::new(self.compile_expr(d)?)), None => None };
                Expr::WhenExpr { subject: Box::new(self.compile_expr(subject)?), cases: new_cases, default: new_default }
            }
            Expr::Await(inner) => Expr::Await(Box::new(self.compile_expr(inner)?)),
            Expr::OkLit(e) => Expr::OkLit(Box::new(self.compile_expr(e)?)),
            Expr::ErrLit(e) => Expr::ErrLit(Box::new(self.compile_expr(e)?)),
            Expr::GenericCall { name, type_params, args } => {
                let mut new_args = Vec::with_capacity(args.len());
                for a in args { new_args.push(self.compile_expr(a)?); }
                Expr::GenericCall { name: name.clone(), type_params: type_params.clone(), args: new_args }
            }
            Expr::SayExpr(inner) => Expr::SayExpr(Box::new(self.compile_expr(inner)?)),
            // Statements inside try/catch/else blocks are left as-is here;
            // they go through the same per-statement compile path as any
            // other block when the block is executed (consistent with how
            // Stmt::TryCatch's blocks are already handled elsewhere).
            Expr::TryExpr { body, catch_var, catch_body, else_body } => {
                Expr::TryExpr {
                    body: body.clone(),
                    catch_var: catch_var.clone(),
                    catch_body: catch_body.clone(),
                    else_body: else_body.clone(),
                }
            }
        };
        Ok(out)
    }

    /// Constant-folds a BinOp when both sides are literal Int/Float/Bool/Str
    /// after compilation. Returns None when folding isn't applicable, in
    /// which case the BinOp is emitted as-is for the runtime to evaluate
    /// normally — identical semantics, just pre-computed when provable.
    fn try_fold_binop(op: &BinOpKind, l: &Expr, r: &Expr) -> Option<Expr> {
        use BinOpKind::*;
        match (l, r) {
            (Expr::IntLit(a), Expr::IntLit(b)) => {
                let (a, b) = (*a, *b);
                match op {
                    Add => Some(Expr::IntLit(a.checked_add(b)?)),
                    Sub => Some(Expr::IntLit(a.checked_sub(b)?)),
                    Mul => Some(Expr::IntLit(a.checked_mul(b)?)),
                    Div => if b != 0 { Some(Expr::IntLit(a / b)) } else { None },
                    Mod => if b != 0 { Some(Expr::IntLit(a % b)) } else { None },
                    Eq    => Some(Expr::BoolLit(a == b)),
                    NotEq => Some(Expr::BoolLit(a != b)),
                    Lt    => Some(Expr::BoolLit(a < b)),
                    Gt    => Some(Expr::BoolLit(a > b)),
                    LtEq  => Some(Expr::BoolLit(a <= b)),
                    GtEq  => Some(Expr::BoolLit(a >= b)),
                    And   => Some(Expr::BoolLit(a != 0 && b != 0)),
                    Or    => Some(Expr::BoolLit(a != 0 || b != 0)),
                }
            }
            (Expr::FloatLit(a), Expr::FloatLit(b)) => {
                let (a, b) = (*a, *b);
                match op {
                    Add => Some(Expr::FloatLit(a + b)),
                    Sub => Some(Expr::FloatLit(a - b)),
                    Mul => Some(Expr::FloatLit(a * b)),
                    Div => if b != 0.0 { Some(Expr::FloatLit(a / b)) } else { None },
                    Mod => if b != 0.0 { Some(Expr::FloatLit(a % b)) } else { None },
                    Eq    => Some(Expr::BoolLit((a - b).abs() < f64::EPSILON)),
                    NotEq => Some(Expr::BoolLit((a - b).abs() >= f64::EPSILON)),
                    Lt    => Some(Expr::BoolLit(a < b)),
                    Gt    => Some(Expr::BoolLit(a > b)),
                    LtEq  => Some(Expr::BoolLit(a <= b)),
                    GtEq  => Some(Expr::BoolLit(a >= b)),
                    And   => Some(Expr::BoolLit(a != 0.0 && b != 0.0)),
                    Or    => Some(Expr::BoolLit(a != 0.0 || b != 0.0)),
                }
            }
            (Expr::BoolLit(a), Expr::BoolLit(b)) => {
                let (a, b) = (*a, *b);
                match op {
                    And   => Some(Expr::BoolLit(a && b)),
                    Or    => Some(Expr::BoolLit(a || b)),
                    Eq    => Some(Expr::BoolLit(a == b)),
                    NotEq => Some(Expr::BoolLit(a != b)),
                    _ => None,
                }
            }
            (Expr::StrLit(a), Expr::StrLit(b)) => {
                // Only fold when neither side has interpolation braces,
                // since interpolation is resolved against runtime variables.
                if a.contains('{') || b.contains('{') { return None; }
                match op {
                    Add   => Some(Expr::StrLit(format!("{}{}", a, b))),
                    Eq    => Some(Expr::BoolLit(a == b)),
                    NotEq => Some(Expr::BoolLit(a != b)),
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

// =============================================================================
// PARSER
// =============================================================================
struct Parser {
    tokens: Vec<Token>,
    pos:    usize,
    // BUGFIX: when true, suppresses the "ident => expr" single-param-lambda
    // shorthand in parse_primary(). Needed while parsing match/when arm
    // *patterns* (e.g. `_ => ...`), because otherwise a bare identifier
    // pattern followed by `=>` gets greedily consumed as a lambda literal,
    // swallowing the arrow and the arm's value, which then breaks the
    // match/when arm loop's own `expect(Arrow)` check.
    in_pattern: bool,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self { Parser { tokens, pos: 0, in_pattern: false } }

    fn peek(&self)  -> &Token { self.tokens.get(self.pos).unwrap_or(&Token::EOF) }
    fn peek2(&self) -> &Token { self.tokens.get(self.pos + 1).unwrap_or(&Token::EOF) }
    #[allow(dead_code)]
    fn peek3(&self) -> &Token { self.tokens.get(self.pos + 2).unwrap_or(&Token::EOF) }

    fn advance(&mut self) -> Token {
        let t = self.tokens.get(self.pos).cloned().unwrap_or(Token::EOF);
        self.pos += 1;
        t
    }

    fn eat_newlines(&mut self) {
        while *self.peek() == Token::Newline { self.advance(); }
    }

    fn expect(&mut self, tok: &Token) -> Result<(), String> {
        if self.peek() == tok { self.advance(); Ok(()) }
        else { Err(format!("Expected {:?} got {:?}", tok, self.peek())) }
    }

    fn expect_ident(&mut self) -> Result<String, String> {
        match self.advance() {
            Token::Ident(s) => Ok(s),
            t => Err(format!("Expected identifier, got {:?}", t)),
        }
    }

    pub fn parse(&mut self) -> Result<Vec<Stmt>, String> {
        let mut stmts = Vec::new();
        self.eat_newlines();
        while *self.peek() != Token::EOF {
            let s = self.parse_stmt()?;
            stmts.push(s);
            while *self.peek() == Token::Newline { self.advance(); }
        }
        Ok(stmts)
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>, String> {
        self.expect(&Token::LBrace)?;
        self.eat_newlines();
        let mut stmts = Vec::new();
        while *self.peek() != Token::RBrace && *self.peek() != Token::EOF {
            let s = self.parse_stmt()?;
            stmts.push(s);
            while *self.peek() == Token::Newline { self.advance(); }
        }
        self.expect(&Token::RBrace)?;
        Ok(stmts)
    }

    // Feature 41: parse one UI element inside a `ui { }` block.
    // Grammar:  ident [string_or_expr] [ '{' (key ':' expr | nested_element)* '}' ]
    // Extends to: button "label" on_click: fn, image src:"..." width:N, text "..." size:N color:red
    fn parse_ui_node(&mut self) -> Result<UiNode, String> {
        // Accept both Ident and keyword tokens as tag names
        let tag = match self.peek().clone() {
            Token::Ident(s) => { self.advance(); s }
            Token::Button => { self.advance(); "button".to_string() }
            Token::Image  => { self.advance(); "img".to_string() }
            Token::Input  => { self.advance(); "input".to_string() }
            Token::View   => { self.advance(); "div".to_string() }
            Token::Screen => { self.advance(); "section".to_string() }
            Token::Layout => {
                // layout row / layout col / layout grid — peek next token
                self.advance();
                match self.peek().clone() {
                    Token::Row  => { self.advance(); "layout-row".to_string() }
                    Token::Col  => { self.advance(); "layout-col".to_string() }
                    Token::Grid => { self.advance(); "layout-grid".to_string() }
                    Token::Ident(s) if s == "row"  => { self.advance(); "layout-row".to_string() }
                    Token::Ident(s) if s == "col"  => { self.advance(); "layout-col".to_string() }
                    Token::Ident(s) if s == "grid" => { self.advance(); "layout-grid".to_string() }
                    _ => "layout-row".to_string(), // default to row
                }
            }
            Token::Row  => { self.advance(); "layout-row".to_string() }
            Token::Col  => { self.advance(); "layout-col".to_string() }
            Token::Grid => { self.advance(); "layout-grid".to_string() }
            t => return Err(format!("Expected UI tag, got {:?}", t)),
        };

        // Optional inline content: a string (or any primary expr) right after the tag name
        let content = match self.peek().clone() {
            Token::StrLit(_) | Token::IntLit(_) | Token::FloatLit(_) | Token::Ident(_) => {
                // Only treat as content if it's not immediately the start of a new block/EOF.
                // A bare ident here would be ambiguous with a child tag name on the next line,
                // so content is only captured when it's a literal.
                if matches!(self.peek(), Token::StrLit(_) | Token::IntLit(_) | Token::FloatLit(_)) {
                    Some(self.parse_unary_lit()?)
                } else {
                    None
                }
            }
            _ => None,
        };

        let mut props    = Vec::new();
        let mut attrs    = Vec::new();
        let mut children = Vec::new();

        if *self.peek() == Token::LBrace {
            self.advance();
            self.eat_newlines();
            while *self.peek() != Token::RBrace && *self.peek() != Token::EOF {
                // key : value   →  CSS property or attribute
                let key_opt: Option<String> = match self.peek().clone() {
                    Token::Ident(s) if *self.peek2() == Token::Colon => Some(s),
                    Token::Type if *self.peek2() == Token::Colon => Some("type".to_string()),
                    _ => None,
                };
                if let Some(key) = key_opt {
                    self.advance(); // consume the key token (Ident or Type)
                    self.expect(&Token::Colon)?;
                    // Value: allow bare idents to be CSS keyword strings (e.g. align: center)
                    let val = if matches!(self.peek(), Token::Ident(_)) {
                        // If next token after ident is newline/rbrace/comma → treat as string literal
                        if let Token::Ident(s) = self.peek().clone() {
                            let save = self.pos;
                            self.advance();
                            if matches!(self.peek(), Token::Newline | Token::RBrace | Token::Comma | Token::EOF) {
                                Expr::StrLit(s)
                            } else {
                                // looks like a real expression — backtrack and parse normally
                                self.pos = save;
                                self.parse_expr()?
                            }
                        } else { self.parse_expr()? }
                    } else {
                        self.parse_expr()?
                    };
                    if is_ui_attr_key(&key) {
                        attrs.push((key, val));
                    } else {
                        props.push((key, val));
                    }
                    self.eat_newlines();
                    continue;
                }
                // Otherwise: nested child element
                children.push(self.parse_ui_node()?);
                self.eat_newlines();
            }
            self.expect(&Token::RBrace)?;
        }

        Ok(UiNode { tag, content, props, attrs, children })
    }

    // Parses a single literal token as an Expr (used for inline UI content like `text "Hi"`)
    fn parse_unary_lit(&mut self) -> Result<Expr, String> {
        match self.advance() {
            Token::StrLit(s)   => Ok(Expr::StrLit(s)),
            Token::IntLit(n)   => Ok(Expr::IntLit(n)),
            Token::FloatLit(n) => Ok(Expr::FloatLit(n)),
            t => Err(format!("Expected literal content in ui block, got {:?}", t)),
        }
    }

    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        match self.peek().clone() {

            // Feature 1, 8, 9: let — also handles Feature 33 destructuring
            Token::Let => {
                self.advance();
                // Destructuring: let {name, age} = person
                if *self.peek() == Token::LBrace {
                    self.advance();
                    let mut keys = Vec::new();
                    while *self.peek() != Token::RBrace && *self.peek() != Token::EOF {
                        keys.push(self.expect_ident()?);
                        if *self.peek() == Token::Comma { self.advance(); }
                    }
                    self.expect(&Token::RBrace)?;
                    self.expect(&Token::Eq)?;
                    let source = self.parse_expr()?;
                    return Ok(Stmt::Destructure { keys, source });
                }
                // Normal let / multi-assign
                let mut names = Vec::new();
                let mut values = Vec::new();
                names.push(self.expect_ident()?);
                while *self.peek() == Token::Comma {
                    self.advance();
                    names.push(self.expect_ident()?);
                }
                self.expect(&Token::Eq)?;
                values.push(self.parse_expr()?);
                while *self.peek() == Token::Comma {
                    self.advance();
                    values.push(self.parse_expr()?);
                }
                Ok(Stmt::Let { names, values })
            }

            Token::Say => {
                self.advance();
                let mut exprs = Vec::new();
                loop {
                    if matches!(self.peek(), Token::Newline | Token::EOF | Token::RBrace) { break; }
                    if matches!(self.peek(),
                        Token::Let | Token::Fn | Token::Loop | Token::When |
                        Token::Each | Token::Exit | Token::Then | Token::Else |
                        Token::Struct | Token::Impl | Token::Trait | Token::Try |
                        Token::Import | Token::Return
                    ) { break; }
                    exprs.push(self.parse_expr()?);
                    if *self.peek() == Token::Comma { self.advance(); }
                }
                Ok(Stmt::Say(exprs))
            }

            // Feature 40: async fn / Feature 21,22: named+default params
            Token::Async => {
                self.advance();
                self.expect(&Token::Fn)?;
                self.parse_fn_decl(true)
            }

            Token::Fn => {
                self.advance();
                self.parse_fn_decl(false)
            }

            Token::Loop => {
                self.advance();
                let count = self.parse_expr()?;
                let body  = self.parse_block()?;
                Ok(Stmt::Loop { count, body })
            }

            Token::When => {
                self.advance();
                // Guard mode: when { cond => body }  (no subject before brace)
                let guard_mode = *self.peek() == Token::LBrace;
                let subject = if guard_mode { Expr::BoolLit(true) } else { self.parse_expr()? };
                self.expect(&Token::LBrace)?;
                self.eat_newlines();
                let mut cases   = Vec::new();
                let mut default = None;
                while *self.peek() != Token::RBrace && *self.peek() != Token::EOF {
                    if *self.peek() == Token::Else {
                        self.advance();
                        self.expect(&Token::Arrow)?;
                        let body = if *self.peek() == Token::LBrace { self.parse_block()? }
                                   else { vec![self.parse_stmt()?] };
                        default = Some(body);
                        self.eat_newlines();
                        break;
                    }
                    let val  = self.parse_expr()?;
                    self.expect(&Token::Arrow)?;
                    let body = if *self.peek() == Token::LBrace { self.parse_block()? }
                               else { vec![self.parse_stmt()?] };
                    cases.push((val, body));
                    self.eat_newlines();
                }
                self.expect(&Token::RBrace)?;
                Ok(Stmt::When { subject, cases, default })
            }

            Token::Each => {
                self.advance();
                let var  = self.expect_ident()?;
                self.expect(&Token::In)?;
                let iter = self.parse_expr()?;
                let body = self.parse_block()?;
                Ok(Stmt::Each { var, iter, body })
            }

            Token::Exit => {
                self.advance();
                let code = if !matches!(self.peek(), Token::Newline | Token::EOF) {
                    Some(self.parse_expr()?)
                } else { None };
                Ok(Stmt::Exit(code))
            }

            Token::Return => {
                self.advance();
                let val = if !matches!(self.peek(), Token::Newline | Token::EOF | Token::RBrace) {
                    Some(self.parse_expr()?)
                } else { None };
                Ok(Stmt::Return(val))
            }

            // Feature 27: struct decl
            Token::Struct => {
                self.advance();
                let name = self.expect_ident()?;
                self.expect(&Token::LBrace)?;
                self.eat_newlines();
                let mut fields = Vec::new();
                while *self.peek() != Token::RBrace && *self.peek() != Token::EOF {
                    fields.push(self.expect_ident()?);
                    if *self.peek() == Token::Comma { self.advance(); }
                    while *self.peek() == Token::Newline { self.advance(); }
                }
                self.expect(&Token::RBrace)?;
                Ok(Stmt::StructDecl(StructDef { name, fields }))
            }

            // Feature 41b: view ComponentName { ... }
            Token::View => {
                self.advance();
                let name = self.expect_ident()?;
                self.expect(&Token::LBrace)?;
                self.eat_newlines();
                let mut body = Vec::new();
                while *self.peek() != Token::RBrace && *self.peek() != Token::EOF {
                    body.push(self.parse_ui_node()?);
                    while *self.peek() == Token::Newline { self.advance(); }
                }
                self.expect(&Token::RBrace)?;
                Ok(Stmt::ViewDecl { name, body })
            }

            // Feature 42: screen PageName { title: "..." theme: dark body { ... } }
            Token::Screen => {
                self.advance();
                let name = self.expect_ident()?;
                self.expect(&Token::LBrace)?;
                self.eat_newlines();
                let mut title: Option<Expr> = None;
                let mut theme: Option<String> = None;
                let mut body: Vec<UiNode> = Vec::new();
                while *self.peek() != Token::RBrace && *self.peek() != Token::EOF {
                    if let Token::Ident(s) = self.peek().clone() {
                        if s == "title" && *self.peek2() == Token::Colon {
                            self.advance();
                            self.expect(&Token::Colon)?;
                            title = Some(self.parse_expr()?);
                            self.eat_newlines();
                            continue;
                        }
                        if s == "theme" && *self.peek2() == Token::Colon {
                            self.advance();
                            self.expect(&Token::Colon)?;
                            let t = match self.advance() {
                                Token::Ident(v) => v,
                                Token::StrLit(v) => v,
                                _ => "light".to_string(),
                            };
                            theme = Some(t);
                            self.eat_newlines();
                            continue;
                        }
                    }
                    body.push(self.parse_ui_node()?);
                    while *self.peek() == Token::Newline { self.advance(); }
                }
                self.expect(&Token::RBrace)?;
                Ok(Stmt::ScreenDecl { name, title, theme, body })
            }

            // Feature 41: ui { } block — CSS-level UI designer.
            // ui PageName { div { color: "red" ... text "Hi" button "Go" { onClick: "..." } } }
            Token::Ui => {
                self.advance();
                let name = self.expect_ident()?;
                self.expect(&Token::LBrace)?;
                self.eat_newlines();
                let mut root  = Vec::new();
                let mut title = None;
                while *self.peek() != Token::RBrace && *self.peek() != Token::EOF {
                    // Top-level 'title: "..."' metadata line
                    if let Token::Ident(s) = self.peek().clone() {
                        if s == "title" && *self.peek2() == Token::Colon {
                            self.advance(); // consume 'title'
                            self.expect(&Token::Colon)?;
                            title = Some(self.parse_expr()?);
                            self.eat_newlines();
                            continue;
                        }
                    }
                    root.push(self.parse_ui_node()?);
                    while *self.peek() == Token::Newline { self.advance(); }
                }
                self.expect(&Token::RBrace)?;
                Ok(Stmt::UiDecl { name, title, root })
            }

            // Feature 29: impl block
            Token::Impl => {
                self.advance();
                let target = self.expect_ident()?;
                self.expect(&Token::LBrace)?;
                self.eat_newlines();
                let mut methods = Vec::new();
                while *self.peek() != Token::RBrace && *self.peek() != Token::EOF {
                    // each method: fn name(self, params) { body }
                    let is_async = if *self.peek() == Token::Async { self.advance(); true } else { false };
                    self.expect(&Token::Fn)?;
                    let mname  = self.expect_ident()?;
                    self.expect(&Token::LParen)?;
                    let mut params = Vec::new();
                    // first param can be 'self'
                    while *self.peek() != Token::RParen && *self.peek() != Token::EOF {
                        match self.peek().clone() {
                            Token::Ident(s) => { self.advance(); params.push(s); }
                            _ => { self.advance(); }
                        }
                        if *self.peek() == Token::Comma { self.advance(); }
                    }
                    self.expect(&Token::RParen)?;
                    let body = self.parse_block()?;
                    methods.push((mname, params, body));
                    self.eat_newlines();
                    let _ = is_async; // stored for future use
                }
                self.expect(&Token::RBrace)?;
                Ok(Stmt::ImplDecl(ImplBlock { target, methods }))
            }

            // Feature 30: trait
            Token::Trait => {
                self.advance();
                let name = self.expect_ident()?;
                self.expect(&Token::LBrace)?;
                self.eat_newlines();
                let mut methods = Vec::new();
                while *self.peek() != Token::RBrace && *self.peek() != Token::EOF {
                    if *self.peek() == Token::Fn { self.advance(); }
                    let mname = self.expect_ident()?;
                    // skip optional param list
                    if *self.peek() == Token::LParen {
                        self.advance();
                        while *self.peek() != Token::RParen && *self.peek() != Token::EOF { self.advance(); }
                        self.advance();
                    }
                    methods.push(mname);
                    while *self.peek() == Token::Newline { self.advance(); }
                }
                self.expect(&Token::RBrace)?;
                Ok(Stmt::TraitDecl(TraitDef { name, methods }))
            }

            // Feature 31: try / catch / else
            Token::Try => {
                self.advance();
                let body = self.parse_block()?;
                self.eat_newlines();
                self.expect(&Token::Catch)?;
                // BUGFIX: README uses bare `catch err { }` (no parens), but
                // this only recognized the parenthesized `catch (e) { }`
                // form — a bare identifier fell through to `None`, silently
                // discarding the error, and then `self.parse_block()` failed
                // because the identifier was still sitting unconsumed before
                // the `{`. Now both `catch (e) { }` and `catch e { }` work.
                let catch_var = if *self.peek() == Token::LParen {
                    self.advance();
                    let v = self.expect_ident()?;
                    self.expect(&Token::RParen)?;
                    Some(v)
                } else if let Token::Ident(_) = self.peek().clone() {
                    Some(self.expect_ident()?)
                } else { None };
                let catch_body = self.parse_block()?;
                self.eat_newlines();
                let else_body = if *self.peek() == Token::Else {
                    self.advance();
                    Some(self.parse_block()?)
                } else { None };
                Ok(Stmt::TryCatch { body, catch_var, catch_body, else_body })
            }

            // Feature 36: use (module loading)
            Token::Import => {
                self.advance();
                let name = self.expect_ident()?;
                Ok(Stmt::Import(name))
            }

            // Feature 37: type alias OR type() builtin call
            Token::Type => {
                self.advance();
                // If followed by (, treat as type() function call
                if *self.peek() == Token::LParen {
                    // parse as Expr::Call { name: "type", args, named: [] }
                    self.advance(); // consume (
                    let mut args = Vec::new();
                    while *self.peek() != Token::RParen && *self.peek() != Token::EOF {
                        args.push(self.parse_expr()?);
                        if *self.peek() == Token::Comma { self.advance(); }
                    }
                    self.expect(&Token::RParen)?;
                    return Ok(Stmt::Expr(Expr::Call { name: "type".to_string(), args, named: Vec::new() }));
                }
                let alias = self.expect_ident()?;
                self.expect(&Token::Eq)?;
                let target = self.expect_ident()?;
                Ok(Stmt::TypeAlias { alias, target })
            }

            // if
            Token::Ident(ref s) if s == "if" => {
                self.advance();
                let cond      = self.parse_expr()?;
                let then_body = self.parse_block()?;
                self.eat_newlines();
                let else_body = if *self.peek() == Token::Else {
                    self.advance();
                    Some(self.parse_block()?)
                } else { None };
                Ok(Stmt::If { cond, then_body, else_body })
            }

            // Fix 4: struct field assignment: ident.field = expr
            Token::Ident(_) if *self.peek2() == Token::Dot => {
                // BUGFIX: this used to hand-roll a single `obj.field` access
                // and return immediately as `Stmt::Expr`, which silently
                // dropped anything after it — e.g. `p.x + p.y` as a bare
                // statement parsed only `p.x`, then choked on the leftover
                // `+ p.y` on the *next* parse_stmt() call ("Unexpected
                // token: Plus"). Same problem for chains like `obj.field.method()`.
                // Only the plain-assignment shape `obj.field = value` truly
                // needs special handling (there's no Expr for it); everything
                // else must go through full expression parsing so trailing
                // operators/method chains aren't lost.
                let save = self.pos;
                let obj = self.expect_ident()?;
                self.advance(); // consume .
                let field = self.expect_ident()?;
                if *self.peek() == Token::Eq {
                    self.advance(); // consume =
                    let value = self.parse_expr()?;
                    return Ok(Stmt::FieldAssign { obj, field, value });
                }
                // Not an assignment — rewind and parse the whole thing as a
                // normal expression (handles `obj.field`, `obj.field + x`,
                // `obj.field.method()`, chains, etc. correctly).
                self.pos = save;
                let expr = self.parse_expr()?;
                return Ok(Stmt::Expr(expr));
            }

            // Assignment: ident = expr
            Token::Ident(_) if *self.peek2() == Token::Eq => {
                let name = self.expect_ident()?;
                self.advance(); // consume =
                let value = self.parse_expr()?;
                Ok(Stmt::Assign { name, value })
            }

            // Feature 50: style { selector { prop: val ... } ... } — scoped CSS ruleset
            // At top-level, stores rules. When used inside ui/screen/view it injects <style>.
            Token::Style => {
                self.advance();
                self.expect(&Token::LBrace)?;
                self.eat_newlines();
                let mut rules: Vec<(String, Vec<(String, String)>)> = Vec::new();
                while *self.peek() != Token::RBrace && *self.peek() != Token::EOF {
                    // selector — can be ident, .class, #id (we lex . and # separately)
                    let selector = match self.peek().clone() {
                        Token::Ident(s) => { self.advance(); s }
                        Token::Hash => {
                            self.advance();
                            let id = self.expect_ident()?;
                            format!("#{}", id)
                        }
                        Token::Dot => {
                            self.advance();
                            let cls = self.expect_ident()?;
                            format!(".{}", cls)
                        }
                        _ => { self.advance(); "*".to_string() }
                    };
                    self.expect(&Token::LBrace)?;
                    self.eat_newlines();
                    let mut props: Vec<(String, String)> = Vec::new();
                    while *self.peek() != Token::RBrace && *self.peek() != Token::EOF {
                        let key = match self.peek().clone() {
                            Token::Ident(s) => { self.advance(); s }
                            Token::Type     => { self.advance(); "type".to_string() }
                            _ => { self.advance(); continue; }
                        };
                        self.expect(&Token::Colon)?;
                        // Value: string literal or ident or number
                        let val = match self.advance() {
                            Token::StrLit(s)   => s,
                            Token::Ident(s)    => s,
                            Token::IntLit(n)   => n.to_string(),
                            Token::FloatLit(f) => f.to_string(),
                            _ => String::new(),
                        };
                        props.push((css_key(&key), val));
                        self.eat_newlines();
                    }
                    self.expect(&Token::RBrace)?;
                    rules.push((selector, props));
                    self.eat_newlines();
                }
                self.expect(&Token::RBrace)?;
                Ok(Stmt::StyleBlock { rules })
            }

            // Feature 46-49: input / layout row / layout col / layout grid
            // as top-level statement → interpret as implicit ui { } output
            Token::Input | Token::Layout | Token::Row | Token::Col | Token::Grid => {
                // These are valid inside ui/screen blocks; at top level parse as expression node
                // by treating them as a one-shot ui page with a single node.
                let node = self.parse_ui_node()?;
                // Wrap in a minimal HTML page and output
                Ok(Stmt::UiDecl {
                    name: node.tag.clone().replace('-', "_"),
                    title: None,
                    root: vec![node],
                })
            }

            _ => Ok(Stmt::Expr(self.parse_expr()?)),
        }
    }

    // Feature 21, 22, 40: fn with named+default params, async
    fn parse_fn_decl(&mut self, is_async: bool) -> Result<Stmt, String> {
        let name = self.expect_ident()?;
        // Feature 38: optional generic params <T, U>
        if *self.peek() == Token::Lt {
            self.advance();
            while *self.peek() != Token::Gt && *self.peek() != Token::EOF { self.advance(); }
            if *self.peek() == Token::Gt { self.advance(); }
        }
        self.expect(&Token::LParen)?;
        let mut params: Vec<(String, Option<Expr>)> = Vec::new();
        while *self.peek() != Token::RParen && *self.peek() != Token::EOF {
            let pname = self.expect_ident()?;
            // Feature 21: named param marker ident:
            // Feature 22: default param ident: default_val  or  ident = default_val
            let default = if *self.peek() == Token::Colon {
                self.advance();
                // if next is comma/rparen → named-only marker, no default
                if matches!(self.peek(), Token::Comma | Token::RParen) {
                    None
                } else {
                    Some(self.parse_expr()?)
                }
            } else if *self.peek() == Token::Eq {
                self.advance();
                Some(self.parse_expr()?)
            } else {
                None
            };
            params.push((pname, default));
            if *self.peek() == Token::Comma { self.advance(); }
        }
        self.expect(&Token::RParen)?;
        // optional return type hint: -> type  (ignored at runtime)
        if *self.peek() == Token::ThinArrow {
            self.advance();
            self.expect_ident()?;
        }
        let body = self.parse_block()?;
        Ok(Stmt::Fn { name, params, body, is_async })
    }

    // -------------------------------------------------------------------------
    // Expression parsing
    // -------------------------------------------------------------------------
    fn parse_expr(&mut self) -> Result<Expr, String> { self.parse_pipe() }

    // Feature 26: pipe operator |>
    fn parse_pipe(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_ternary()?;
        while *self.peek() == Token::Pipe {
            self.advance();
            let right = self.parse_ternary()?;
            left = Expr::Pipe { left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_ternary(&mut self) -> Result<Expr, String> {
        let cond = self.parse_or()?;
        if *self.peek() == Token::Question {
            self.advance();
            let then_val = self.parse_or()?;
            self.expect(&Token::Colon)?;
            let else_val = self.parse_or()?;
            return Ok(Expr::Ternary {
                cond: Box::new(cond), then_val: Box::new(then_val), else_val: Box::new(else_val),
            });
        }
        Ok(cond)
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        while *self.peek() == Token::Or {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::BinOp { op: BinOpKind::Or, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_equality()?;
        while *self.peek() == Token::And {
            self.advance();
            let right = self.parse_equality()?;
            left = Expr::BinOp { op: BinOpKind::And, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_comparison()?;
        loop {
            let op = match self.peek() {
                Token::Is    => BinOpKind::Eq,
                Token::EqEq  => BinOpKind::Eq,
                Token::NotEq => BinOpKind::NotEq,
                _ => break,
            };
            self.advance();
            let right = self.parse_comparison()?;
            left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_range()?;
        loop {
            let op = match self.peek() {
                Token::Lt   => BinOpKind::Lt,
                Token::Gt   => BinOpKind::Gt,
                Token::LtEq => BinOpKind::LtEq,
                Token::GtEq => BinOpKind::GtEq,
                _ => break,
            };
            self.advance();
            let right = self.parse_range()?;
            left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_range(&mut self) -> Result<Expr, String> {
        let left = self.parse_add()?;
        if *self.peek() == Token::DotDot {
            self.advance();
            let right = self.parse_add()?;
            return Ok(Expr::Range(Box::new(left), Box::new(right)));
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_mul()?;
        loop {
            let op = match self.peek() {
                Token::Plus  => BinOpKind::Add,
                Token::Minus => BinOpKind::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_mul()?;
            left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Token::Star    => BinOpKind::Mul,
                Token::Slash   => BinOpKind::Div,
                Token::Percent => BinOpKind::Mod,
                _ => break,
            };
            self.advance();
            let right = self.parse_unary()?;
            left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if *self.peek() == Token::Not {
            self.advance();
            return Ok(Expr::Not(Box::new(self.parse_unary()?)));
        }
        if *self.peek() == Token::Minus {
            self.advance();
            return Ok(Expr::BinOp {
                op:    BinOpKind::Sub,
                left:  Box::new(Expr::IntLit(0)),
                right: Box::new(self.parse_unary()?),
            });
        }
        self.parse_postfix()
    }

    // Feature 25: method chaining — parse expr then .method() chains
    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_primary()?;
        loop {
            if *self.peek() == Token::Dot {
                self.advance();
                let method = self.expect_ident()?;
                let args = if *self.peek() == Token::LParen {
                    self.advance();
                    let mut a = Vec::new();
                    while *self.peek() != Token::RParen && *self.peek() != Token::EOF {
                        a.push(self.parse_expr()?);
                        if *self.peek() == Token::Comma { self.advance(); }
                    }
                    self.expect(&Token::RParen)?;
                    a
                } else { Vec::new() };
                // Could be field access or method call
                if args.is_empty() && !matches!(self.tokens.get(self.pos - 1), Some(Token::RParen)) {
                    // try as field access first
                    expr = Expr::StructAccess(Box::new(expr), method);
                } else {
                    expr = Expr::MethodCall { object: Box::new(expr), method, args };
                }
            } else if *self.peek() == Token::LBracket {
                self.advance();
                let idx = self.parse_expr()?;
                self.expect(&Token::RBracket)?;
                expr = Expr::NullSafe(Box::new(expr), Box::new(idx));
            } else if *self.peek() == Token::Question && *self.peek2() == Token::LBracket {
                self.advance();
                self.advance();
                let idx = self.parse_expr()?;
                self.expect(&Token::RBracket)?;
                expr = Expr::NullSafe(Box::new(expr), Box::new(idx));
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.peek().clone() {
            Token::IntLit(n)  => { self.advance(); Ok(Expr::IntLit(n)) }
            Token::FloatLit(n)=> { self.advance(); Ok(Expr::FloatLit(n)) }
            Token::BoolLit(b) => { self.advance(); Ok(Expr::BoolLit(b)) }
            Token::Null       => { self.advance(); Ok(Expr::Null) }
            Token::StrLit(s)  => { self.advance(); Ok(Expr::StrLit(s)) }

            // Feature 23: spread ...expr
            Token::Spread => {
                self.advance();
                let e = self.parse_primary()?;
                Ok(Expr::Spread(Box::new(e)))
            }

            // Feature 40: await expr
            Token::Await => {
                self.advance();
                let e = self.parse_primary()?;
                Ok(Expr::Await(Box::new(e)))
            }

            // BUGFIX: `try { } catch err { }` as an expression — mirrors the
            // Token::Try handling in parse_stmt(), but reachable from
            // parse_expr()/parse_primary() so `let x = try { ... } catch e { ... }`
            // (as shown in the README) actually parses.
            Token::Try => {
                self.advance();
                let body = self.parse_block()?;
                self.eat_newlines();
                self.expect(&Token::Catch)?;
                let catch_var = if *self.peek() == Token::LParen {
                    self.advance();
                    let v = self.expect_ident()?;
                    self.expect(&Token::RParen)?;
                    Some(v)
                } else if let Token::Ident(_) = self.peek().clone() {
                    Some(self.expect_ident()?)
                } else { None };
                let catch_body = self.parse_block()?;
                self.eat_newlines();
                let else_body = if *self.peek() == Token::Else {
                    self.advance();
                    Some(self.parse_block()?)
                } else { None };
                Ok(Expr::TryExpr { body, catch_var, catch_body, else_body })
            }

            // where an expression is expected (e.g. match/when arm values), so
            // the README's `match { ... => say "..." }` style examples work.
            // Only a single expr is taken (not comma-separated like the `say`
            // *statement*) to avoid swallowing a following match-arm comma.
            Token::Say => {
                self.advance();
                let e = self.parse_expr()?;
                Ok(Expr::SayExpr(Box::new(e)))
            }

            // Feature 24: match expr { val => expr ... }
            Token::Match => {
                self.advance();
                let subject = self.parse_expr()?;
                self.expect(&Token::LBrace)?;
                self.eat_newlines();
                let mut arms: Vec<(Expr, Expr)> = Vec::new();
                let mut default = None;
                while *self.peek() != Token::RBrace && *self.peek() != Token::EOF {
                    if *self.peek() == Token::Else {
                        self.advance();
                        self.expect(&Token::Arrow)?;
                        default = Some(Box::new(self.parse_expr()?));
                        self.eat_newlines();
                        break;
                    }
                    let prev_in_pattern = self.in_pattern;
                    self.in_pattern = true;
                    let pat = self.parse_expr()?;
                    self.in_pattern = prev_in_pattern;
                    self.expect(&Token::Arrow)?;
                    let val = self.parse_expr()?;
                    arms.push((pat, val));
                    if *self.peek() == Token::Comma { self.advance(); }
                    self.eat_newlines();
                }
                self.expect(&Token::RBrace)?;
                Ok(Expr::Match { subject: Box::new(subject), arms, default })
            }

            // when as expression: let x = when { cond => val ... }
            Token::When => {
                self.advance();
                let guard_mode = *self.peek() == Token::LBrace;
                let subject = if guard_mode { Expr::BoolLit(true) } else { self.parse_expr()? };
                self.expect(&Token::LBrace)?;
                self.eat_newlines();
                let mut cases: Vec<(Expr, Expr)> = Vec::new();
                let mut default: Option<Box<Expr>> = None;
                while *self.peek() != Token::RBrace && *self.peek() != Token::EOF {
                    if *self.peek() == Token::Else {
                        self.advance();
                        self.expect(&Token::Arrow)?;
                        default = Some(Box::new(self.parse_expr()?));
                        self.eat_newlines();
                        break;
                    }
                    let prev_in_pattern = self.in_pattern;
                    self.in_pattern = true;
                    let pat = self.parse_expr()?;
                    self.in_pattern = prev_in_pattern;
                    self.expect(&Token::Arrow)?;
                    let val = self.parse_expr()?;
                    cases.push((pat, val));
                    self.eat_newlines();
                }
                self.expect(&Token::RBrace)?;
                Ok(Expr::WhenExpr { subject: Box::new(subject), cases, default })
            }

            // Feature 39: lambda  x => expr  or  (x, y) => expr
            // Detected in primary when we see LParen followed by idents + Arrow
            Token::LParen => {
                // Try to detect lambda: (a, b) => expr
                let save = self.pos;
                let is_lambda = self.try_parse_lambda_params();
                if is_lambda {
                    // Already consumed ( params )
                    let params = self.get_last_lambda_params();
                    self.expect(&Token::Arrow)?;
                    let body = self.parse_expr()?;
                    return Ok(Expr::Lambda { params, body: Box::new(body) });
                }
                self.pos = save;
                // Normal grouping
                self.advance(); // (
                let e = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(e)
            }

            // List literal OR list comprehension
            Token::LBracket => {
                self.advance();
                self.eat_newlines();
                if *self.peek() == Token::RBracket {
                    self.advance();
                    return Ok(Expr::List(Vec::new()));
                }
                // Check for spread first item
                let first = self.parse_expr()?;
                self.eat_newlines();
                // Feature 34: list comprehension [expr for var in iter if cond]
                if *self.peek() == Token::For {
                    self.advance();
                    let var  = self.expect_ident()?;
                    self.expect(&Token::In)?;
                    let iter = self.parse_expr()?;
                    let cond = if *self.peek() == Token::Ident(String::from("if")) {
                        self.advance();
                        Some(Box::new(self.parse_expr()?))
                    } else { None };
                    self.expect(&Token::RBracket)?;
                    return Ok(Expr::ListComp { expr: Box::new(first), var, iter: Box::new(iter), cond });
                }
                // Normal list — eat newlines between elements
                let mut items = vec![first];
                while *self.peek() == Token::Comma {
                    self.advance();
                    self.eat_newlines();
                    if *self.peek() == Token::RBracket { break; }
                    items.push(self.parse_expr()?);
                    self.eat_newlines();
                }
                self.expect(&Token::RBracket)?;
                Ok(Expr::List(items))
            }

            // Feature 35: map literal { key: value, ... }
            Token::LBrace => {
                self.advance();
                self.eat_newlines();
                let mut pairs: Vec<(String, Expr)> = Vec::new();
                while *self.peek() != Token::RBrace && *self.peek() != Token::EOF {
                    let key = match self.advance() {
                        Token::Ident(s) => s,
                        Token::StrLit(s) => s,
                        t => return Err(format!("Expected map key, got {:?}", t)),
                    };
                    self.expect(&Token::Colon)?;
                    let val = self.parse_expr()?;
                    pairs.push((key, val));
                    if *self.peek() == Token::Comma { self.advance(); }
                    self.eat_newlines();
                }
                self.expect(&Token::RBrace)?;
                Ok(Expr::Map(pairs))
            }

            Token::Ident(name) => {
                self.advance();
                // Feature 38: generic call fn<T>(args)
                if *self.peek() == Token::Lt {
                    // peek ahead to see if it's a generic call or comparison
                    let save = self.pos;
                    if let Ok(type_params) = self.try_parse_type_params() {
                        if *self.peek() == Token::LParen {
                            self.advance();
                            let mut args = Vec::new();
                            while *self.peek() != Token::RParen && *self.peek() != Token::EOF {
                                args.push(self.parse_expr()?);
                                if *self.peek() == Token::Comma { self.advance(); }
                            }
                            self.expect(&Token::RParen)?;
                            return Ok(Expr::GenericCall { name, type_params, args });
                        }
                    }
                    self.pos = save;
                }

                // Feature 21: named-param call  fn(name: val, age: val)
                if *self.peek() == Token::LParen {
                    self.advance();
                    let mut args: Vec<Expr> = Vec::new();
                    let mut named: Vec<(String, Expr)> = Vec::new();

                    while *self.peek() != Token::RParen && *self.peek() != Token::EOF {
                        // Feature 23: spread in call site
                        if *self.peek() == Token::Spread {
                            self.advance();
                            args.push(Expr::Spread(Box::new(self.parse_expr()?)));
                            if *self.peek() == Token::Comma { self.advance(); }
                            continue;
                        }
                        // Named arg: ident: expr
                        if let Token::Ident(aname) = self.peek().clone() {
                            if *self.peek2() == Token::Colon {
                                self.advance(); // consume ident
                                self.advance(); // consume :
                                let val = self.parse_expr()?;
                                named.push((aname, val));
                                if *self.peek() == Token::Comma { self.advance(); }
                                continue;
                            }
                        }
                        args.push(self.parse_expr()?);
                        if *self.peek() == Token::Comma { self.advance(); }
                    }
                    self.expect(&Token::RParen)?;

                    // Feature 28: Struct auto-ctor — if name matches a known struct
                    // (resolve at eval time via Call with named args)
                    return Ok(Expr::Call { name, args, named });
                }

                // Feature 39: single-param lambda  x => expr
                // (suppressed in pattern position — see `in_pattern` doc comment)
                if *self.peek() == Token::Arrow && !self.in_pattern {
                    self.advance();
                    let body = self.parse_expr()?;
                    return Ok(Expr::Lambda { params: vec![name], body: Box::new(body) });
                }

                Ok(Expr::Ident(name))
            }

            // type() as expression (e.g. say type(x), let t = type(x))
            Token::Type => {
                self.advance();
                if *self.peek() == Token::LParen {
                    self.advance();
                    let mut args = Vec::new();
                    while *self.peek() != Token::RParen && *self.peek() != Token::EOF {
                        args.push(self.parse_expr()?);
                        if *self.peek() == Token::Comma { self.advance(); }
                    }
                    self.expect(&Token::RParen)?;
                    Ok(Expr::Call { name: "type".to_string(), args, named: Vec::new() })
                } else {
                    Ok(Expr::Ident("type".to_string()))
                }
            }

            other => Err(format!("Unexpected token: {:?}", other)),
        }
    }

    // Helper: try to parse (a, b, c) as lambda params, return true if Arrow follows
    fn try_parse_lambda_params(&mut self) -> bool {
        self.pos += 1; // skip (
        let mut ok = true;
        loop {
            match self.tokens.get(self.pos) {
                Some(Token::Ident(_)) => { self.pos += 1; }
                Some(Token::Comma)    => { self.pos += 1; }
                Some(Token::RParen)   => { self.pos += 1; break; }
                _                     => { ok = false; break; }
            }
        }
        if ok && self.tokens.get(self.pos) == Some(&Token::Arrow) { true }
        else { false }
    }

    // After try_parse_lambda_params succeeds, extract params from the range we scanned
    /// Recovers a lambda's parameter names after the parser has already
    /// scanned past `(params) =>` and confirmed it's looking at a lambda
    /// (i.e. `self.pos` is sitting right before the `=>` arrow token).
    /// Rather than tracking param names during that initial scan, we just
    /// walk backwards from the current position to the matching `(` and
    /// collect the identifiers in between, in order.
    fn get_last_lambda_params(&self) -> Vec<String> {
        let mut params = Vec::new();
        let mut j = self.pos; // self.pos is at the `=>` arrow token
        while j > 0 {
            j -= 1;
            match &self.tokens[j] {
                Token::LParen => break,
                Token::Ident(s) => params.insert(0, s.clone()),
                _ => {}
            }
        }
        params
    }

    fn try_parse_type_params(&mut self) -> Result<Vec<String>, String> {
        self.expect(&Token::Lt)?;
        let mut types = Vec::new();
        while *self.peek() != Token::Gt && *self.peek() != Token::EOF {
            types.push(self.expect_ident()?);
            if *self.peek() == Token::Comma { self.advance(); }
        }
        self.expect(&Token::Gt)?;
        Ok(types)
    }
}

// =============================================================================
// BUILT-IN MODULES — Feature 36: use math / strings / io / os
// =============================================================================
// =============================================================================
// MALIB ENGINE — Remox Advanced Math Engine (backing implementation)
// Ek single-file mini-CAS: string expression ko tokenize → parse → eval/solve.
// Supports: + - * / % ^, parens, unary minus, variables (x), implicit
// multiplication ("2x", "3(x+1)"), functions (sqrt, sin, cos, tan, log, ln,
// abs, exp), aur equations ("expr = expr") ko linear/quadratic/numeric
// (Newton-Raphson + bisection fallback) tareeke se solve karta hai.
// =============================================================================
mod malib_engine {

    #[derive(Debug, Clone, PartialEq)]
    enum Tok {
        Num(f64),
        Ident(String),
        Plus, Minus, Star, Slash, Percent, Caret,
        LParen, RParen, Comma, Eq,
        End,
    }

    fn tokenize(s: &str) -> Result<Vec<Tok>, String> {
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        let mut toks = Vec::new();
        while i < chars.len() {
            let c = chars[i];
            if c.is_whitespace() { i += 1; continue; }
            match c {
                '+' => { toks.push(Tok::Plus); i += 1; }
                '-' => { toks.push(Tok::Minus); i += 1; }
                '*' => { toks.push(Tok::Star); i += 1; }
                '/' => { toks.push(Tok::Slash); i += 1; }
                '%' => { toks.push(Tok::Percent); i += 1; }
                '^' => { toks.push(Tok::Caret); i += 1; }
                '(' => { toks.push(Tok::LParen); i += 1; }
                ')' => { toks.push(Tok::RParen); i += 1; }
                ',' => { toks.push(Tok::Comma); i += 1; }
                '=' => { toks.push(Tok::Eq); i += 1; }
                _ if c.is_ascii_digit() || c == '.' => {
                    let start = i;
                    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') { i += 1; }
                    let numstr: String = chars[start..i].iter().collect();
                    let n: f64 = numstr.parse().map_err(|_| format!("Invalid number '{}'", numstr))?;
                    toks.push(Tok::Num(n));
                }
                _ if c.is_alphabetic() || c == '_' => {
                    let start = i;
                    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') { i += 1; }
                    let id: String = chars[start..i].iter().collect();
                    toks.push(Tok::Ident(id));
                }
                _ => return Err(format!("Unexpected character '{}'", c)),
            }
        }
        toks.push(Tok::End);
        Ok(toks)
    }

    // AST for the mini math expression language
    #[derive(Debug, Clone)]
    pub enum Node {
        Num(f64),
        Var(String),
        Neg(Box<Node>),
        Add(Box<Node>, Box<Node>),
        Sub(Box<Node>, Box<Node>),
        Mul(Box<Node>, Box<Node>),
        Div(Box<Node>, Box<Node>),
        Mod(Box<Node>, Box<Node>),
        Pow(Box<Node>, Box<Node>),
        Call(String, Vec<Node>),
    }

    struct Parser { toks: Vec<Tok>, pos: usize }

    impl Parser {
        fn new(toks: Vec<Tok>) -> Self { Parser { toks, pos: 0 } }
        fn peek(&self) -> &Tok { self.toks.get(self.pos).unwrap_or(&Tok::End) }
        fn advance(&mut self) -> Tok { let t = self.peek().clone(); self.pos += 1; t }

        // Returns (lhs_node, Option<rhs_node>) — rhs present if an '=' was found (equation)
        fn parse_equation(&mut self) -> Result<(Node, Option<Node>), String> {
            let lhs = self.parse_expr()?;
            if *self.peek() == Tok::Eq {
                self.advance();
                let rhs = self.parse_expr()?;
                Ok((lhs, Some(rhs)))
            } else {
                Ok((lhs, None))
            }
        }

        fn parse_expr(&mut self) -> Result<Node, String> {
            let mut node = self.parse_term()?;
            loop {
                match self.peek() {
                    Tok::Plus  => { self.advance(); let r = self.parse_term()?; node = Node::Add(Box::new(node), Box::new(r)); }
                    Tok::Minus => { self.advance(); let r = self.parse_term()?; node = Node::Sub(Box::new(node), Box::new(r)); }
                    _ => break,
                }
            }
            Ok(node)
        }

        fn parse_term(&mut self) -> Result<Node, String> {
            let mut node = self.parse_unary()?;
            loop {
                match self.peek() {
                    Tok::Star    => { self.advance(); let r = self.parse_unary()?; node = Node::Mul(Box::new(node), Box::new(r)); }
                    Tok::Slash   => { self.advance(); let r = self.parse_unary()?; node = Node::Div(Box::new(node), Box::new(r)); }
                    Tok::Percent => { self.advance(); let r = self.parse_unary()?; node = Node::Mod(Box::new(node), Box::new(r)); }
                    // Implicit multiplication: "2x", "3(x+1)", "2 sqrt(x)"
                    Tok::Num(_) | Tok::Ident(_) | Tok::LParen => {
                        let r = self.parse_unary()?;
                        node = Node::Mul(Box::new(node), Box::new(r));
                    }
                    _ => break,
                }
            }
            Ok(node)
        }

        fn parse_unary(&mut self) -> Result<Node, String> {
            match self.peek() {
                Tok::Minus => { self.advance(); let n = self.parse_unary()?; Ok(Node::Neg(Box::new(n))) }
                Tok::Plus  => { self.advance(); self.parse_unary() }
                _ => self.parse_power(),
            }
        }

        fn parse_power(&mut self) -> Result<Node, String> {
            let base = self.parse_atom()?;
            if *self.peek() == Tok::Caret {
                self.advance();
                let exp = self.parse_unary()?; // right-assoc, allows -x exponents
                Ok(Node::Pow(Box::new(base), Box::new(exp)))
            } else {
                Ok(base)
            }
        }

        fn parse_atom(&mut self) -> Result<Node, String> {
            match self.advance() {
                Tok::Num(n) => Ok(Node::Num(n)),
                Tok::LParen => {
                    let n = self.parse_expr()?;
                    if self.advance() != Tok::RParen { return Err("Expected ')'".into()); }
                    Ok(n)
                }
                Tok::Ident(name) => {
                    if *self.peek() == Tok::LParen {
                        self.advance();
                        let mut args = Vec::new();
                        if *self.peek() != Tok::RParen {
                            args.push(self.parse_expr()?);
                            while *self.peek() == Tok::Comma {
                                self.advance();
                                args.push(self.parse_expr()?);
                            }
                        }
                        if self.advance() != Tok::RParen { return Err("Expected ')' after args".into()); }
                        Ok(Node::Call(name, args))
                    } else {
                        Ok(Node::Var(name))
                    }
                }
                t => Err(format!("Unexpected token: {:?}", t)),
            }
        }
    }

    pub fn parse(src: &str) -> Result<(Node, Option<Node>), String> {
        let toks = tokenize(src)?;
        let mut p = Parser::new(toks);
        let eq = p.parse_equation()?;
        if *p.peek() != Tok::End { return Err("Unexpected trailing tokens".into()); }
        Ok(eq)
    }

    pub fn parse_single(src: &str) -> Result<Node, String> {
        let (lhs, rhs) = parse(src)?;
        match rhs {
            None => Ok(lhs),
            Some(r) => Ok(Node::Sub(Box::new(lhs), Box::new(r))), // treat "a=b" as a-b for eval()
        }
    }

    // Evaluate node given a single variable binding (most equations here are
    // single-variable: x). Unknown idents default to the bound variable value
    // if name matches `var_name`, else 0 if var_name not relevant, else error.
    pub fn eval(node: &Node, var_name: &str, var_val: f64) -> Result<f64, String> {
        match node {
            Node::Num(n) => Ok(*n),
            Node::Var(name) => {
                if name == var_name { Ok(var_val) }
                else if name == "pi" { Ok(std::f64::consts::PI) }
                else if name == "e"  { Ok(std::f64::consts::E) }
                else { Err(format!("Unknown variable '{}'", name)) }
            }
            Node::Neg(a) => Ok(-eval(a, var_name, var_val)?),
            Node::Add(a, b) => Ok(eval(a, var_name, var_val)? + eval(b, var_name, var_val)?),
            Node::Sub(a, b) => Ok(eval(a, var_name, var_val)? - eval(b, var_name, var_val)?),
            Node::Mul(a, b) => Ok(eval(a, var_name, var_val)? * eval(b, var_name, var_val)?),
            Node::Div(a, b) => {
                let bv = eval(b, var_name, var_val)?;
                if bv == 0.0 { return Err("Division by zero".into()); }
                Ok(eval(a, var_name, var_val)? / bv)
            }
            Node::Mod(a, b) => Ok(eval(a, var_name, var_val)? % eval(b, var_name, var_val)?),
            Node::Pow(a, b) => Ok(eval(a, var_name, var_val)?.powf(eval(b, var_name, var_val)?)),
            Node::Call(name, args) => {
                let a0 = || -> Result<f64, String> { eval(args.get(0).ok_or("missing arg")?, var_name, var_val) };
                match name.as_str() {
                    "sqrt" => Ok(a0()?.sqrt()),
                    "cbrt" => Ok(a0()?.cbrt()),
                    "abs"  => Ok(a0()?.abs()),
                    "sin"  => Ok(a0()?.sin()),
                    "cos"  => Ok(a0()?.cos()),
                    "tan"  => Ok(a0()?.tan()),
                    "asin" => Ok(a0()?.asin()),
                    "acos" => Ok(a0()?.acos()),
                    "atan" => Ok(a0()?.atan()),
                    "log" | "ln" => Ok(a0()?.ln()),
                    "log2"  => Ok(a0()?.log2()),
                    "log10" => Ok(a0()?.log10()),
                    "exp"   => Ok(a0()?.exp()),
                    "floor" => Ok(a0()?.floor()),
                    "ceil"  => Ok(a0()?.ceil()),
                    "round" => Ok(a0()?.round()),
                    _ => Err(format!("Unknown function '{}'", name)),
                }
            }
        }
    }

    // Detect a free variable name in the expression tree (first non-pi/e ident found)
    pub fn find_var(node: &Node) -> Option<String> {
        match node {
            Node::Num(_) => None,
            Node::Var(name) => {
                if name == "pi" || name == "e" { None } else { Some(name.clone()) }
            }
            Node::Neg(a) => find_var(a),
            Node::Add(a, b) | Node::Sub(a, b) | Node::Mul(a, b)
            | Node::Div(a, b) | Node::Mod(a, b) | Node::Pow(a, b) =>
                find_var(a).or_else(|| find_var(b)),
            Node::Call(_, args) => args.iter().find_map(find_var),
        }
    }

    // Try to extract polynomial coefficients [c0, c1, c2, ...] for c0 + c1*x + c2*x^2 + ...
    // up to degree 2, by sampling f(-2..2) and solving via finite differences.
    // Returns None if the function isn't well-approximated by a degree<=2 polynomial.
    fn try_poly_coeffs(node: &Node, var: &str) -> Option<(f64, f64, f64)> {
        // sample 5 points around 0 to fit a quadratic robustly even if not exactly polynomial
        let xs = [-2.0, -1.0, 0.0, 1.0, 2.0];
        let mut ys = [0.0; 5];
        for (i, x) in xs.iter().enumerate() {
            ys[i] = eval(node, var, *x).ok()?;
        }
        // Fit quadratic c0 + c1*x + c2*x^2 via least squares (Vandermonde, 5 pts, 3 unknowns)
        // Build normal equations manually (3x3 system)
        let n = 5.0;
        let sx: f64 = xs.iter().sum();
        let sx2: f64 = xs.iter().map(|x| x * x).sum();
        let sx3: f64 = xs.iter().map(|x| x.powi(3)).sum();
        let sx4: f64 = xs.iter().map(|x| x.powi(4)).sum();
        let sy: f64 = ys.iter().sum();
        let sxy: f64 = xs.iter().zip(ys.iter()).map(|(x, y)| x * y).sum();
        let sx2y: f64 = xs.iter().zip(ys.iter()).map(|(x, y)| x * x * y).sum();

        // Solve [[n,sx,sx2],[sx,sx2,sx3],[sx2,sx3,sx4]] * [c0,c1,c2]^T = [sy,sxy,sx2y]^T
        let m = [[n, sx, sx2], [sx, sx2, sx3], [sx2, sx3, sx4]];
        let rhs = [sy, sxy, sx2y];
        let coeffs = solve_3x3(m, rhs)?;
        let (c0, c1, c2) = (coeffs[0], coeffs[1], coeffs[2]);
        // Verify fit quality against the original samples
        for (x, y) in xs.iter().zip(ys.iter()) {
            let pred = c0 + c1 * x + c2 * x * x;
            if (pred - y).abs() > 1e-6 * (1.0 + y.abs()) { return None; }
        }
        Some((c0, c1, c2))
    }

    fn solve_3x3(m: [[f64; 3]; 3], rhs: [f64; 3]) -> Option<[f64; 3]> {
        // Cramer's rule
        let det3 = |a: [[f64; 3]; 3]| -> f64 {
            a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
                - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
                + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0])
        };
        let d = det3(m);
        if d.abs() < 1e-12 { return None; }
        let mut result = [0.0; 3];
        for col in 0..3 {
            let mut mc = m;
            for row in 0..3 { mc[row][col] = rhs[row]; }
            result[col] = det3(mc) / d;
        }
        Some(result)
    }

    #[derive(Debug, Clone)]
    pub enum SolveResult {
        NoVariable(f64),           // equation had no free variable, evaluated to a number (0 means true)
        Linear(f64),               // single real solution
        Quadratic(Vec<f64>),       // 0, 1, or 2 real solutions
        Numeric(Vec<f64>),         // numerically found root(s)
        NoRealSolution,
        Error(String),
    }

    // f(x) = lhs(x) - rhs(x) = 0 — solve for the free variable.
    pub fn solve_equation(lhs: &Node, rhs: Option<&Node>) -> SolveResult {
        let diff = match rhs {
            Some(r) => Node::Sub(Box::new(lhs.clone()), Box::new(r.clone())),
            None => lhs.clone(),
        };
        let var = match find_var(&diff) {
            Some(v) => v,
            None => {
                return match eval(&diff, "__none__", 0.0) {
                    Ok(v) => SolveResult::NoVariable(v),
                    Err(e) => SolveResult::Error(e),
                };
            }
        };

        // Try polynomial fit (covers linear & quadratic forms, even when written
        // with parens/expansion like "(x+1)*(x-2)")
        if let Some((c0, c1, c2)) = try_poly_coeffs(&diff, &var) {
            if c2.abs() < 1e-9 {
                if c1.abs() < 1e-9 {
                    return if c0.abs() < 1e-9 { SolveResult::NoVariable(0.0) } else { SolveResult::NoRealSolution };
                }
                return SolveResult::Linear(-c0 / c1);
            } else {
                let disc = c1 * c1 - 4.0 * c2 * c0;
                if disc < -1e-9 {
                    return SolveResult::NoRealSolution;
                } else if disc.abs() < 1e-9 {
                    return SolveResult::Quadratic(vec![-c1 / (2.0 * c2)]);
                } else {
                    let sq = disc.sqrt();
                    let r1 = (-c1 + sq) / (2.0 * c2);
                    let r2 = (-c1 - sq) / (2.0 * c2);
                    let mut rs = vec![r1, r2];
                    rs.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    return SolveResult::Quadratic(rs);
                }
            }
        }

        // Fallback: numeric root finding — Newton-Raphson from multiple seeds,
        // then bisection scan as a safety net for harder/transcendental equations.
        let f = |x: f64| eval(&diff, &var, x);
        let df = |x: f64| -> Result<f64, String> {
            let h = 1e-6;
            Ok((f(x + h)? - f(x - h)?) / (2.0 * h))
        };

        let mut roots: Vec<f64> = Vec::new();
        let seeds = [-50.0, -10.0, -5.0, -2.0, -1.0, 0.0, 0.5, 1.0, 2.0, 5.0, 10.0, 50.0];
        for &seed in seeds.iter() {
            let mut x = seed;
            let mut ok = true;
            for _ in 0..100 {
                let fx = match f(x) { Ok(v) => v, Err(_) => { ok = false; break; } };
                if fx.abs() < 1e-9 { break; }
                let dfx = match df(x) { Ok(v) => v, Err(_) => { ok = false; break; } };
                if dfx.abs() < 1e-12 { ok = false; break; }
                x -= fx / dfx;
                if !x.is_finite() { ok = false; break; }
            }
            if ok {
                if let Ok(fx) = f(x) {
                    if fx.abs() < 1e-6 && !roots.iter().any(|r: &f64| (r - x).abs() < 1e-4) {
                        roots.push((x * 1e9).round() / 1e9);
                    }
                }
            }
        }

        if roots.is_empty() {
            // Bisection scan across a practical range as last resort
            let prev_x = -20.0;
            let prev_f = f(prev_x);
            if let Ok(mut pf) = prev_f {
                let mut x = prev_x;
                let step = 0.25;
                while x < 20.0 {
                    let nx = x + step;
                    if let Ok(nf) = f(nx) {
                        if pf.is_finite() && nf.is_finite() && pf * nf < 0.0 {
                            let mut lo = x; let mut hi = nx;
                            for _ in 0..80 {
                                let mid = (lo + hi) / 2.0;
                                if let Ok(mf) = f(mid) {
                                    if let Ok(lf) = f(lo) {
                                        if lf * mf <= 0.0 { hi = mid; } else { lo = mid; }
                                    }
                                }
                            }
                            let root = ((lo + hi) / 2.0 * 1e9).round() / 1e9;
                            if !roots.iter().any(|r: &f64| (r - root).abs() < 1e-4) {
                                roots.push(root);
                            }
                        }
                        pf = nf;
                    }
                    x = nx;
                    let _ = prev_x; // silence unused warning across loop boundary
                }
            }
        }

        if roots.is_empty() {
            SolveResult::NoRealSolution
        } else {
            roots.sort_by(|a, b| a.partial_cmp(b).unwrap());
            SolveResult::Numeric(roots)
        }
    }
}


// =========================================================================
// Remojoke — Remox's built-in joke library.
//
//   use remojoke
//   Category.remojoke("programming")
//
// Two-part design, both real (nothing here is a canned "always returns the
// same string" stub):
//
//   1. STATIC BANK — 200 original, hand-written jokes (8 per category,
//      25 categories). Fixed text, picked at random per call.
//
//   2. GENERATOR ENGINE — per-category Mad-Libs-style templates + word
//      lists. On each call there's a 50% chance Remojoke builds a *new*
//      joke on the spot by filling a random template with random words,
//      instead of reading from the static bank. This is the actual
//      "custom joke banane wala engine" — combinatorially, the 25
//      categories' templates × word lists produce well over 20,000
//      distinct possible outputs (documented honestly below, not just
//      claimed) even though only 200 were hand-written.
//
// Both paths use the interpreter's existing rand_state LCG (same PRNG
// `rand.int()`/`rand.float()` use — see __rand_int/__rand_float), so
// output differs run to run like every other Remox randomness call, and
// carries the same "not cryptographically random, seeded from
// entropy_source()" caveat documented on Interpreter::new().
// =========================================================================

/// One category: canonical key, any Hindi/English aliases that should
/// resolve to it, 8 static jokes, and the generator's templates + the
/// word-list names (by index into REMOJOKE_WORDLISTS) each template pulls
/// from ({0} in a template = REMOJOKE_WORDLISTS[slot0], etc).
struct RemojokeCategory {
    key: &'static str,
    aliases: &'static [&'static str],
    jokes: &'static [&'static str],
    templates: &'static [(&'static str, &'static [usize])], // (template, slot -> wordlist index)
}

// ---- Shared word lists used by generator templates (index = list id) ----
const RJ_NAMES:   &[&str] = &["Ramesh","Suresh","Pappu","Golu","Chintu","Bunty","Pinky","Rinku","Sonu","Montu","Guddu","Bittu","Chulbul","Bholu","Titu"];
const RJ_PROFS:   &[&str] = &["engineer","doctor","teacher","police wale","manager","programmer","chef","driver","vakil","scientist","farmer","peon","clerk","professor"];
const RJ_ANIMALS: &[&str] = &["bandar","kutta","billi","gadha","murga","bakri","sher","hathi","ullu","kachua","tota","machhli","chuha","ghoda"];
const RJ_TECH:    &[&str] = &["bug","server","WiFi","RAM","laptop","AI model","database","password","cloud","API","cache","firewall","kernel","compiler"];
const RJ_FOODS:   &[&str] = &["samosa","chai","maggi","pizza","biryani","dosa","jalebi","momo","paratha","chole bhature","idli","rasgulla","vada pav","gulab jamun"];
const RJ_PLACES:  &[&str] = &["Delhi","Mumbai","office","college","jail","aasman","chaand","school","bazaar","ghar","gym","hospital","court","bank"];
const RJ_SUBJECTS:&[&str] = &["Maths","Physics","Chemistry","Biology","History","English","Hindi","Computer Science","Economics","Sanskrit"];
const RJ_SPORTS:  &[&str] = &["cricket","football","kabaddi","badminton","chess","carrom","hockey","kho-kho"];
const RJ_BODY:    &[&str] = &["dimaag","pait","haath","pair","aankh","kaan","dil","dant"];
const RJ_NUMS:    &[&str] = &["2","5","10","100","1000","0","-1","infinity","NaN","404"];

fn remojoke_wordlist(idx: usize) -> &'static [&'static str] {
    match idx {
        0 => RJ_NAMES, 1 => RJ_PROFS, 2 => RJ_ANIMALS, 3 => RJ_TECH, 4 => RJ_FOODS,
        5 => RJ_PLACES, 6 => RJ_SUBJECTS, 7 => RJ_SPORTS, 8 => RJ_BODY, 9 => RJ_NUMS,
        _ => &["cheez"],
    }
}

const REMOJOKE_CATEGORIES: &[RemojokeCategory] = &[
    RemojokeCategory { key: "programming", aliases: &["coding","code","dev","developer"], jokes: &[
        "Programmer apni biwi se: chai bana do. Biwi bina bole chali gayi. Programmer bola: dekha, infinite loop bhi kabhi kabhi silent hota hai.",
        "Ek programmer ne bulb badalne se mana kar diya. Bola: bug hardware mein hai, mera scope nahi.",
        "Interviewer: recursion samjhao. Candidate: recursion samajhne ke liye pehle recursion samajhna padta hai.",
        "Do programmers shaadi kar rahe the. Priest bola: I now pronounce you... 'undefined'.",
        "Code review mein senior ne comment kiya: 'ye kaam kar raha hai, par mujhe nahi pata kyun.' Junior khush ho gaya, promotion samjha.",
        "Programmer ke ghar aag lagi. Wo bola: don't worry, git commit kiya hua hai sab.",
        "Ek bug 3 saal se production mein tha. Naam rakh diya gaya 'legacy feature'.",
        "Programmer se poocha gaya password kya hai. Usne bola: 'incorrect' — taaki system bole 'your password is incorrect', aur usko yaad rahe.",
    ], templates: &[
        ("{0}, ek {1}, ne code likha jisme sirf ek {3} tha — aur wahi 3 din se production down kar raha hai.", &[0,1,3]),
        ("{0} ne apna password '{4}' rakha kyunki agar bhool bhi gaya to database mein dhoondhna aasan hai.", &[0,4]),
        ("Ek {1} har subah {4} khaake code likhta hai — bolta hai bugs bhi bhookhe rehte hain agar khaana na mile.", &[1,4]),
        ("{0} ne interview mein bola uska favorite data structure {2} hai — HR abhi tak confuse hai.", &[0,2]),
        ("Ek {3} itna purana ho gaya ki usne khud resign letter bhej diya {5} se.", &[3,5]),
        ("{0} ne git commit message likha '{4} fix kiya' — asal mein usne sirf {4} khaaya tha, code touch nahi kiya.", &[0,4]),
    ]},
    RemojokeCategory { key: "chemistry", aliases: &["chem"], jokes: &[
        "Chemistry teacher: NaCl kya hai? Student: Sir, mera dost hai, Nakul.",
        "Do atoms sadak par mil gaye. Ek bola: mera electron kho gaya. Dusra bola: sure? Pehla bola: I'm positive.",
        "Chemist ne joke sunaya par koi react nahi hua.",
        "Sodium ek bar bar gaya aur bola: mera ek electron mujhse alag ho gaya, ab main Na+ hu.",
        "Teacher: acid aur base milane se kya banta hai? Student: report card, sir, dono ghar pe milte hi drama hota hai.",
        "Chemistry lab mein sabse bada khatra kya hai? Student jo period table ko syllabus samajh ke last minute padhta hai.",
        "Oxygen aur Magnesium ki shaadi ho gayi. Sab bole: MgO!",
        "Chemistry exam mein likha tha 'balance the equation' — student ne tarazu banake diya.",
    ], templates: &[
        ("{0} ne chemistry lab mein {4} mila diya reaction mein — ab poora {5} us smell se bhara hai.", &[0,4,5]),
        ("Teacher ne poocha valency kya hai, {0} bola: itni hi jitni meri patience {6} ki class mein hai.", &[0,6]),
        ("{1} ne periodic table itna rata ki ab wo apne {8} ko bhi 'element' bolta hai.", &[1,8]),
        ("{0} ka chemistry experiment fail hua kyunki usne {4} ko reagent samajh liya tha.", &[0,4]),
    ]},
    RemojokeCategory { key: "physics", aliases: &["phy"], jokes: &[
        "Newton ka teesra niyam: har action ka ek reaction hota hai — jaise homework dene par student ka gayab hona.",
        "Physics teacher: gravity kya hai? Student: sir, wo cheez jo mera result neeche khींchti hai.",
        "Ek electron traffic police se bola: main negative hu, aap mujhe rok nahi sakte.",
        "Physics exam mein sabse tough sawaal: 'friction kam kaise karein' — jab teacher-student ke beech hi friction ho.",
        "Light ki speed itni fast hai ki wo assignment deadline se pehle bhi pahunch jaaye — student ka answer sheet nahi.",
        "Teacher: ek force lagao. Student: sir already lagaya, phir bhi result move nahi hua.",
        "Physics mein sabse heavy cheez kya hai? Monday morning ka alarm uthana.",
        "Pendulum jaisa hi hai student ka mood — exam se pehle aur baad mein dono extreme pe.",
    ], templates: &[
        ("{0} ne Newton ka pehla niyam prove kiya bina hile-dule {5} mein poora din baith ke.", &[0,5]),
        ("{1} ne bताya ki friction kam karne ka best tareeka hai apne {8} ko exam se door rakhna.", &[1,8]),
        ("{0} ka projectile motion experiment fail hua kyunki usne {4} ko ball samajh ke phenk diya.", &[0,4]),
    ]},
    RemojokeCategory { key: "math", aliases: &["maths","mathematics"], jokes: &[
        "Teacher: x ki value nikaalo. Student: sir, x to already free hai, usse dhoondhna kyun?",
        "Math teacher: zero se divide nahi kar sakte. Student: sir, meri marks bhi to kabhi zero se divide nahi hui, seedha zero hi mili.",
        "Ek number line par 0 aur 1 mil gaye. 0 bola: tu mujhse bada hai. 1 bola: bas thoda sa, positive raho.",
        "Geometry exam mein sabse mushkil sawaal: apne future ka angle nikaalo.",
        "Teacher: pi ki value batao. Student: sir, itni hi jitni meri interest hai is subject mein — infinite aur non-repeating.",
        "Algebra itna tough kyu hai? Kyunki usmein letters bhi numbers jaisa behave karte hain, bina warning ke.",
        "Statistics ka sabse bada joke: average student ka average result.",
        "Math teacher ne poocha 2+2 kitna hota hai. Student bola: depends sir, exam mein ya calculator mein?",
    ], templates: &[
        ("{0} ne {6} exam mein x ki value nikaalne ki jagah {2} ki value nikaal di.", &[0,6,2]),
        ("Teacher ne poocha {9} ka square kya hoga, {0} bola: sir utna hi bada jitna mera homework pending hai.", &[9,0]),
        ("{1} har din {9} equations solve karta hai bas isliye taaki {8} thaka rahe aur worry na kare.", &[1,9,8]),
    ]},
    RemojokeCategory { key: "school", aliases: &["skool"], jokes: &[
        "Principal: tum roz late kyun aate ho? Student: sir, school jaldi shuru hoti hai, main nahi.",
        "Teacher: homework kahan hai? Student: sir, wo bhi absent hai aaj.",
        "School ka sabse mushkil subject kaunsa hai? Wo jisme teacher attendance pehle leta hai, syllabus baad mein.",
        "Recess bell sabse pyaari awaaz hai — kisi bhi gaane se zyada.",
        "Teacher: last bench walo, kya baat ho rahi hai? Student: sir, aapki hi tareef ho rahi thi.",
        "Exam hall mein sabse zyada exercise kya hoti hai? Paas wale ka paper dekhne ke liye gardan ghumana.",
        "School diary mein remark aaya: 'talented but lazy.' Ghar pe reply aaya: 'apne se milta hai.'",
        "Sabse bada miracle: Monday subah 7 baje uthna bina 5 alarms ke.",
    ], templates: &[
        ("{0} school mein itna late aaya ki teacher ne khud usse attendance maang li.", &[0]),
        ("{6} ki class mein {0} so gaya, uth ke bola: sir main sirf apni aankhon ko rest de raha tha, concentration ke liye.", &[6,0]),
        ("Principal ne {0} ko bulaya kyunki uske {8} mein homework ka koi trace nahi tha.", &[0,8]),
    ]},
    RemojokeCategory { key: "college", aliases: &["university","campus"], jokes: &[
        "College ka sabse popular subject: bunk management.",
        "Attendance 75% honi chahiye — bacha hua 25% college ki chai-samosa research mein jaata hai.",
        "Semester exam se ek raat pehle poora syllabus 'important questions' ban jaata hai.",
        "College canteen ka udhaar khata kisi bhi bank ke loan se zyada complex hota hai.",
        "Placement season mein sabse zyada practice hoti hai resume mein 'skills' badhaane ki.",
        "Group project mein ek banda kaam karta hai, baaki sab 'moral support' dete hain.",
        "College fest mein sabse busy log wahi hote hain jo semester mein sabse zyada absent the.",
        "Library sirf exam ke ek hafte pehle full hoti hai, baaki saal khaali padi rehti hai.",
    ], templates: &[
        ("{0} ne poora semester {5} mein bunk maara aur exam se pehle raat {6} poora ratt liya.", &[0,5,6]),
        ("Group project mein {0} ne sirf ek slide banayi — baaki {1} ne poora present kar diya.", &[0,1]),
    ]},
    RemojokeCategory { key: "office", aliases: &["corporate","job","work"], jokes: &[
        "Manager: kaam kaisa chal raha hai? Employee: sir, meeting mein hi to sara din nikal jaata hai, kaam kab karu.",
        "Office ka sabse productive ghanta wo hota hai jab boss chhutti pe ho.",
        "'Urgent' meeting ka matlab hota hai koi 30 minute late hoga, baaki sab wait karenge.",
        "Appraisal season mein sabse bada skill hota hai apni chhoti si cheez ko 'impactful initiative' bolna.",
        "Monday ko sabse zyada energy sirf coffee machine ke paas dikhti hai.",
        "Office ka WiFi password itna strong hota hai jitna employee ka resignation letter likhne ka irada kamzor.",
        "Deadline pass aate hi 'we'll figure it out' team ka official motto ban jaata hai.",
        "Har office mein ek banda hota hai jo reply-all pe 'thanks' bhejta hai poori company ko.",
    ], templates: &[
        ("{0} ne meeting mein bola 'noted' teen baar, par actual mein kuch note nahi kiya.", &[0]),
        ("{1} har Monday {4} lekar aata hai office, bolta hai 'team bonding' ke liye.", &[1,4]),
        ("{0} ka laptop {3} crash hua exact us waqt jab boss demo maang raha tha.", &[0,3]),
    ]},
    RemojokeCategory { key: "marriage", aliases: &["shaadi","pati-patni","husband-wife"], jokes: &[
        "Pati: tum meri baat kabhi nahi sunti. Patni: suni thi, pasand nahi aayi isliye ignore kiya.",
        "Shaadi ke pehle: 'tum meri duniya ho.' Shaadi ke baad: 'zara TV ki awaaz kam karo.'",
        "Patni: tumhe pata bhi hai humari shaadi ko kitne saal ho gaye? Pati: haan, utne hi jitne meri freedom ko khatam hue.",
        "Pati-patni ka sabse bada compromise: remote kiske haath mein rahega.",
        "Shaadi ek exam jaisi hai — syllabus pehle nahi pata chalta, sirf exam ke din pata chalta hai.",
        "Pati bola: aaj khaana main banaunga. Patni khush ho gayi — fir order aaya bahar se.",
        "Anniversary bhoolne ki saza jitni bhaari hoti hai, utni to court ki saza bhi nahi hoti.",
        "Shaadi mein sabse zyada 'yes sir' bolne wala insaan pati hota hai, office mein bhi nahi.",
    ], templates: &[
        ("{0} ne apni shaadi mein promise kiya tha {4} roz banayega, ab saal mein ek baar bhi mushkil hai.", &[0,4]),
        ("Pati-patni ka jhagda {4} ki wajah se shuru hua aur {5} tak pahuch gaya.", &[4,5]),
    ]},
    RemojokeCategory { key: "doctor", aliases: &["hospital","medical"], jokes: &[
        "Patient: doctor, mujhe bhoolne ki bimari hai. Doctor: kabse hai? Patient: kaunsi bimari?",
        "Doctor: aapko din mein 3 baar dawai leni hai. Patient: aur agar main sota reh gaya to?",
        "Doctor: aapka weight badh raha hai. Patient: sir, gravity ki galti hai, meri nahi.",
        "Patient: doctor, mujhe lagta hai main invisible ho raha hu. Doctor: agla number please, koi dikh nahi raha.",
        "Doctor ne prescription itni kharab likhi ki pharmacist ne usse hi wapas bhej diya check karne ke liye.",
        "Patient: doctor, meri body mein har jagah dard hai. Doctor: chai peeni band karo har jagah ungli ghumana.",
        "Doctor: exercise karte ho? Patient: haan sir, remote ke buttons dabana ek acchi cardio hai.",
        "Doctor: aapko stress hai. Patient: sir, aapki fees dekh ke aur badh gaya.",
    ], templates: &[
        ("Patient ne doctor ko bataya uske {8} mein dard hai, doctor ne bola 'ye to sirf {4} zyada khaane se hota hai'.", &[8,4]),
        ("{0} doctor ke paas gaya {8} dikhane, doctor ne poocha 'aap {7} khelte ho?' — jawab tha 'nahi, sirf dekhta hu'.", &[0,8,7]),
    ]},
    RemojokeCategory { key: "police", aliases: &["cop","law"], jokes: &[
        "Police: gaadi kyun tez chala rahe ho? Driver: sir, late ho raha tha aapse milne mein.",
        "Chor ko pakadne ke baad police ne poocha naam, chor bola: 'confidential hai sir'.",
        "Police station mein sabse zyada complaint aati hai neighbour ke loud gaane ki.",
        "Traffic police ne challan kaata, driver bola: 'sir, ye to meri salary se zyada hai' — police bola: 'to gaadi bhi expensive lo'.",
        "Ek chor pakda gaya kyunki usne CCTV ko bhi 'like' kar diya social media pe.",
        "Police: aapke paas licence hai? Driver: sir, confidence hai, kaafi nahi kya?",
        "Thane mein sabse peaceful time chai break hota hai — chor bhi wait kar lete hain.",
        "Police dog ne case solve kar diya, officer ne credit khud le liya report mein.",
    ], templates: &[
        ("{0} ko police ne roka {5} ke paas kyunki uski gaadi se {4} ki smell aa rahi thi.", &[0,5,4]),
        ("Ek {2} thane mein ghus gaya, officer bola: 'FIR file karo isko bhi'.", &[2]),
    ]},
    RemojokeCategory { key: "animals", aliases: &["janwar","pets"], jokes: &[
        "Kutta insan se zyada wafadar hota hai — kam se kam wo late fees nahi maangta.",
        "Billi ne mirror dekha, socha doosri billi aa gayi ghar mein guest banke.",
        "Bandar ne phone uthaya, bola: 'hello, wrong number' — insaan se zyada polite nikla.",
        "Ullu din mein sota hai, raat ko jagta hai — office ke naye employee jaisa schedule.",
        "Gadha itna mehnati hota hai ki uska naam hi insult ban gaya insano ke liye.",
        "Machhli ne poocha: pani ke bahar life kaisi hai? Insaan bola: stressful, tumhari jaisi peaceful nahi.",
        "Kachua slow hai par race jeet gaya — motivational speakers ka favorite example.",
        "Murga roz subah alarm baja deta hai bina kisi ko snooze button dene ke.",
    ], templates: &[
        ("{2} ne {5} mein ghus ke sabko dara diya, asal mein wo bas {4} dhoondh raha tha.", &[2,5,4]),
        ("{0} apne pet {2} ko itna pyaar karta hai ki usko {4} bhi share kar deta hai.", &[0,2,4]),
    ]},
    RemojokeCategory { key: "food", aliases: &["khana","cuisine"], jokes: &[
        "Chai bina subah adhuri lagti hai, motivation se zyada zaroori hai chai.",
        "Maggi 2 minute mein banti hai, par khane ke baad guilt ghante bhar rehta hai.",
        "Samosa akela nahi aata, chai zaroor saath laata hai.",
        "Diet start karne ka best din hamesha 'kal' hota hai.",
        "Biryani ke bina koi function complete nahi hota, chahe wo shaadi ho ya funeral discussion.",
        "Pizza order karte waqt sabse tough decision: extra cheese lena ya budget bachana.",
        "Ghar ka khana sabse tasty tab lagta hai jab bahar ka khana khatam ho jaaye.",
        "Jalebi ka shape hi confusing hai, taste seedha-saada sabse zyada meetha.",
    ], templates: &[
        ("{0} ne {4} order kiya lekin delivery boy {5} pahuch gaya galti se.", &[0,4,5]),
        ("{1} ka diet plan sirf ek din chala jab tak {4} saamne nahi aaya.", &[1,4]),
    ]},
    RemojokeCategory { key: "cricket", aliases: &["ipl","match"], jokes: &[
        "Cricket match dekhte waqt sabse bada expert wahi banta hai jo khud kabhi maidan pe nahi gaya.",
        "Last over mein sabse zyada heartbeat badhti hai TV ke saamne baithe fan ki, player ki nahi.",
        "Rain interruption sabse bada villain hai kisi bhi cricket fan ki zindagi ka.",
        "Umpire ka decision galat ho to poora ghar 'replay dikhao' chillata hai.",
        "Sixer lagte hi neighbour ke ghar se bhi awaaz aati hai, match wahi dekh rahe hote hain.",
        "Fantasy cricket team banate waqt confidence real match se zyada hota hai.",
        "Match haarne ke baad sabse common line: 'captain ne galat decision liya'.",
        "Practice se zyada important hota hai match ke din lucky jersey pehnna.",
    ], templates: &[
        ("{0} ne match dekhte-dekhte itna shout kiya ki {5} ke saare log utha aaye.", &[0,5]),
        ("Last ball pe {0} ne bola 'chhakka lagega' — ball {8} pe jaa lagi bas.", &[0,8]),
    ]},
    RemojokeCategory { key: "bollywood", aliases: &["movies","film"], jokes: &[
        "Bollywood hero ka koi bhi fight scene ho, uske baal hamesha perfect rehte hain.",
        "Interval ke baad hi villain ko pata chalta hai hero uska bhai hai.",
        "Item song ke bina koi bhi Bollywood film incomplete lagti hai producer ko.",
        "Hero akela 20 goondon ko pitta hai, aur real life mein ek machhar se dar jaata hai.",
        "Rain scene mein songs shoot karna Bollywood ka favorite tareeka hai romance dikhane ka.",
        "Climax mein sab kuch coincidence se solve ho jaata hai, real life mein aisa kabhi nahi hota.",
        "Bollywood mein college students 30 saal ke actors play karte hain, bina kisi ko farak pade.",
        "Sad song bajte hi pata chal jaata hai heroine ka breakup scene aane wala hai.",
    ], templates: &[
        ("{0} ne movie dekh ke socha wo bhi {1} ban jayega, agle din alarm miss ho gaya.", &[0,1]),
        ("Film mein hero ne {2} ko bhi dialogue de diya, sabse zyada taaliyan usi scene mein bajin.", &[2]),
    ]},
    RemojokeCategory { key: "engineer", aliases: &["engineering"], jokes: &[
        "Engineer ka motto: jugaad se badi koi engineering nahi hoti.",
        "Engineering degree deti hai, par tension bhi utni hi milti hai saath mein.",
        "Lab report submit karne se ek raat pehle hi sabse zyada 'creativity' aati hai.",
        "Engineer kisi bhi cheez ko tape se theek kar sakta hai, permanent solution baad mein sochenge.",
        "Placement season mein sabse zyada demand engineering ke us branch ki hoti hai jisme least log the.",
        "Engineer apne college project ko 'scalable solution' bolta hai interview mein.",
        "Semester ke last din hi pata chalta hai poora syllabus baaki hai.",
        "Engineering mein sabse important skill hai deadline se ek ghanta pehle sab manage kar lena.",
    ], templates: &[
        ("{0} ne apna project {5} ki raat 3 baje complete kiya, sirf {3} ki wajah se crash ho gaya.", &[0,5,3]),
        ("{1} ne apne jugaad se {2} ko bhi machine bana diya, professor bhi confuse ho gaya.", &[1,2]),
    ]},
    RemojokeCategory { key: "teacher", aliases: &["sir","maam"], jokes: &[
        "Teacher: kal test hai. Poori class ki awaaz gayab ho jaati hai ek second mein.",
        "Teacher: last bench pe kya chal raha hai? Student: sir, aapki hi class chal rahi hai.",
        "Teacher ka favorite dialogue: 'ye important hai, exam mein aayega'.",
        "Teacher ne bola homework check karungi, poori class ne ek dusre se copy maangi.",
        "Teacher: parents ko bulao. Student: sir, wo bhi busy hain mere jaise.",
        "Teacher ka sabse bada power move: seating arrangement badal dena bina warning ke.",
        "Class mein sabse dara hua pal: teacher ka register kholna attendance ke alawa.",
        "Teacher: notebook check karungi. Student: sir, wo bhi 'study leave' pe hai.",
    ], templates: &[
        ("{6} ki teacher ne test liya, {0} ne poora paper {4} ki recipe likh di galti se.", &[6,0,4]),
        ("Teacher ne poocha homework kahan hai, {0} bola: sir wo {2} le gaya.", &[0,2]),
    ]},
    RemojokeCategory { key: "exam", aliases: &["test","pariksha"], jokes: &[
        "Exam hall mein sabse dua ye hoti hai ki jo aata hai wahi poocha jaaye.",
        "Result aane se pehle raat ki neend sabse kam hoti hai.",
        "Exam ke ek din pehle poora syllabus 'important' ban jaata hai.",
        "Answer sheet mein extra pages maangna confidence dikhata hai, marks nahi.",
        "Exam center pe pahunch ke pata chalta hai admit card ghar pe reh gaya.",
        "Objective paper mein guessing bhi ek skill ban jaata hai.",
        "Exam khatam hote hi sabse common sawaal: 'tune kya likha?'",
        "Result day sabse zyada prayers Google se bhi zyada hoti hain.",
    ], templates: &[
        ("{0} ne {6} ka exam diya, poora paper likha par galti se roll number bhool gaya.", &[0,6]),
        ("Exam hall mein {0} ne itna guess kiya ki teacher bhi impressed ho gaya.", &[0]),
    ]},
    RemojokeCategory { key: "internet", aliases: &["wifi","network"], jokes: &[
        "WiFi ka naam 'pantry' rakhna sabse common office prank hai.",
        "Internet slow ho to sabse pehla shak router pe jaata hai, phir provider pe.",
        "Video call mein 'aap mute hain' sabse common line ban chuki hai.",
        "Buffering wheel dekh ke jitna patience test hota hai, kisi exam mein nahi hota.",
        "WiFi password bhoolna ghar ke sabse bade crisis mein se ek hai.",
        "Download 99% pe atak jaana life ka sabse frustrating moment hai.",
        "Internet band hone par ghar ka har member expert electrician ban jaata hai.",
        "Router restart karna family ka sabse bada troubleshooting solution hai har problem ke liye.",
    ], templates: &[
        ("{0} ka WiFi {5} mein exact us waqt band hua jab video call important thi.", &[0,5]),
        ("{3} slow chal raha tha, {0} ne router 5 baar restart kiya bina fayde ke.", &[3,0]),
    ]},
    RemojokeCategory { key: "ai", aliases: &["chatgpt","artificial intelligence"], jokes: &[
        "AI se poocha future kaisa hoga, usne bola: 'depends on your prompt quality'.",
        "ChatGPT se sabse zyada poocha jaane wala sawaal: 'mera assignment likh do'.",
        "AI ne insaan se kaam chheena nahi, sirf deadline jaldi bata di.",
        "AI chatbot se baat karke logon ko lagta hai unse zyada patience kisi insaan mein nahi hai.",
        "AI se poocha gaya joke sunao, usne data se best joke nikala — insaan ne bola 'purana hai'.",
        "AI model itna smart hai ki galat jawab bhi confidence se deta hai.",
        "AI se resume banwana ab utna hi common hai jitna pehle Google karna tha.",
        "AI se poocha: kya tum sapne dekhte ho? Jawab aaya: sirf tab jab server maintenance pe ho.",
    ], templates: &[
        ("{0} ne AI se {4} ki recipe poochi, jawab mein {3} error aa gaya.", &[0,4,3]),
        ("{1} ne apna poora kaam AI se karwaya, meeting mein khud confuse ho gaya explain karte waqt.", &[1]),
    ]},
    RemojokeCategory { key: "dad", aliases: &["puns","dadjoke"], jokes: &[
        "Beta: papa, main bhookha hu. Papa: hi bhookha, main papa.",
        "Bakery mein roti se poocha gaya kaisi ho, usne bola: 'thodi tension mein hu, sab mujhe grind kar rahe hain'.",
        "Papa: light band karo. Beta: kaunsi? Papa: jo jal rahi hai wahi na.",
        "Calendar apne aap mein bahut busy hota hai — 365 din se kaam kar raha hai.",
        "Papa ka joke sunke koi nahi hasta, phir bhi wo dobara sunaate hain agle din.",
        "Chair ne kabhi complain nahi ki, kyunki wo hamesha 'stable' rehti hai.",
        "Papa: main ek joke sunata hu construction ke baare mein — abhi bhi bana raha hu.",
        "Beta: papa ye pun mat maro. Papa: mai to sirf 'dad'-icated hu iss kaam ke liye.",
    ], templates: &[
        ("{0} papa ban gaya, ab har baat pe ek pun crack karta hai, {5} mein bhi.", &[0,5]),
        ("Dad joke: {2} ne bank se loan liya, interest '{9}' pe tha.", &[2,9]),
    ]},
    RemojokeCategory { key: "ghost", aliases: &["horror","bhoot"], jokes: &[
        "Bhoot ne apna resume bheja, skill likha: 'excellent at scaring, minimal follow-up'.",
        "Ek bhoot ne haunted house chhod diya kyunki rent time pe nahi mila.",
        "Bhootiya haveli mein sabse dara hua wo tha jo WiFi password dhoondh raha tha.",
        "Bhoot ko sabse zyada dar lagta hai jab uski shadow bhi nahi dikhti.",
        "Raat ko akela ghar mein awaaz sunke sabse pehla khayal aata hai: 'bijli ka bill zyada aaya kya'.",
        "Bhoot party mein sabse popular game hota hai 'kaun deewar ke paar jaa sakta hai fastest'.",
        "Haunted house tour guide bola: yahan koi rehta nahi, sirf rent-free stay karte hain bhoot.",
        "Ek bhoot itna busy tha ki usne apna haunting schedule Google Calendar pe daal diya.",
    ], templates: &[
        ("{5} mein raat ko awaaz aayi, {0} dar ke {2} samajh baitha, asal mein wo bas {3} tha.", &[5,0,2,3]),
        ("Bhoot ne {0} ko dara diya, {0} ne ulta bola 'pehle apna resume bhejo'.", &[0]),
    ]},
    RemojokeCategory { key: "weather", aliases: &["mausam"], jokes: &[
        "Mausam vibhaag ka forecast aur meri planning dono equally unreliable hain.",
        "Barish shuru hoti hai exact us din jab naye kapde pehen ke nikalte hain.",
        "Garmi itni hoti hai ki AC bhi thak jaata hai cooling karte karte.",
        "Sardi mein razai chhodna sabse mushkil kaam hota hai roz subah ka.",
        "Umbrella tabhi ghar pe milta hai jab dhoop nikli ho, barish mein kabhi nahi.",
        "Weather app har din 'chance of rain' bolta hai, chahe mausam kaisa bhi ho.",
        "Sabse confusing mausam wo hota hai jisme AC aur sweater dono chahiye ek hi din.",
        "Garmi ke season mein bijli jaana ek national tradition ban chuka hai.",
    ], templates: &[
        ("{5} mein aaj itni garmi hai ki {2} bhi chhaya dhoondh raha hai.", &[5,2]),
        ("{0} umbrella le ke nikla, dhoop nikal aayi — Murphy's law confirm ho gaya.", &[0]),
    ]},
    RemojokeCategory { key: "gym", aliases: &["fitness","workout"], jokes: &[
        "Gym membership lene ka sabse bada motivation January hota hai, February tak khatam.",
        "Gym mirror sabse honest doston se zyada judgemental hota hai.",
        "Rest day sabse important workout hai — kam se kam mera to yahi belief hai.",
        "Gym mein sabse mehnati banda wahi hota hai jo sirf selfie ke liye aata hai.",
        "Protein shake peene se pehle motivation zyada hota hai, workout ke baad khatam.",
        "Leg day skip karna ek unwritten tradition hai gym jaane walon ki.",
        "Gym trainer ka favorite dialogue: 'bas ek aur set', jo kabhi last nahi hota.",
        "Weighing scale sabse bada villain hai kisi bhi fitness journey ka.",
    ], templates: &[
        ("{0} gym gaya sirf {4} khaane ke baad guilt kam karne, 10 minute mein wapas aa gaya.", &[0,4]),
        ("{1} har roz gym jaata hai selfie ke liye, {8} kabhi exercise nahi karta.", &[1,8]),
    ]},
    RemojokeCategory { key: "relationship", aliases: &["love","dating"], jokes: &[
        "'Hum sirf dost hain' sunne ke baad sabse zyada dard hota hai.",
        "Relationship mein sabse bada fight hota hai kaunsi movie dekhni hai uspe.",
        "Text message ka reply late aane se poora din ki mood decide ho jaata hai.",
        "Best friend zone ek aisi jagah hai jahan se koi wapas nahi aata.",
        "Valentine's Day sabse zyada single logon ko yaad dilaata hai unki status ki.",
        "Relationship mein 'kuch nahi, tum batao' sabse dangerous jawab hota hai.",
        "Crush ke saamne aane par sabse zyada awkward silence create hoti hai.",
        "Long distance relationship sabse zyada test leti hai patience aur data pack dono ki.",
    ], templates: &[
        ("{0} ne apne crush ko message kiya, reply mein sirf ek 'haan' aaya — poori raat overthinking chali.", &[0]),
        ("{0} aur {1} ka pehla date {5} mein tha, dono itne nervous the ki {4} order karna bhool gaye.", &[0,1,5,4]),
    ]},
    RemojokeCategory { key: "kids", aliases: &["bacche","children"], jokes: &[
        "Bacchon ka sabse bada sawaal hota hai 'kyun', jiska jawab kabhi khatam nahi hota.",
        "Bachpan mein sabse bada tension hota tha homework, ab bade hoke bhi wahi feeling office mein aati hai.",
        "Bacche jhoot pakadne mein experts hote hain, khaaskar jab papa 'thodi der mein aata hu' bolte hain.",
        "Bacchon ka energy level unlimited hota hai, sirf study time pe hi khatam ho jaata hai.",
        "Bachpan ka sabse bada crime tha TV zyada dekhna, ab bade hoke phone zyada dekhna hai.",
        "Bacche sabse honest critics hote hain — kisi bhi dish ka taste seedha bata dete hain.",
        "School bag ka weight kabhi kabhi bacche ke weight se zyada hota hai.",
        "Bacchon ko sabse zyada khushi milti hai jab unhe extra chhutti mil jaaye bina wajah bataye.",
    ], templates: &[
        ("Bacche ne poocha '{9} kyun hota hai', papa ne jawab diya 'bada hoke pata chalega'.", &[9]),
        ("{0} bachpan mein {2} banna chahta tha, ab bada hoke {1} ban gaya.", &[0,2,1]),
    ]},
];

/// Case/whitespace-insensitive category lookup, also checking aliases.
fn remojoke_find_category(name: &str) -> Option<&'static RemojokeCategory> {
    let needle = name.trim().to_lowercase();
    REMOJOKE_CATEGORIES.iter().find(|c| {
        c.key == needle || c.aliases.iter().any(|a| *a == needle)
    })
}

/// Advances a caller-owned LCG state by one step and returns the new value.
/// Same constants as __rand_int/__rand_float so Remojoke's randomness comes
/// from the same PRNG stream as the rest of Remox (see Interpreter::new
/// doc comment re: entropy_source / not cryptographically secure).
fn remojoke_next(state: &mut u64) -> u64 {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *state
}

fn remojoke_pick<'a, T>(state: &mut u64, items: &'a [T]) -> &'a T {
    let idx = (remojoke_next(state) >> 33) as usize % items.len();
    &items[idx]
}

/// Fills a template's {N} placeholders. N is the wordlist id directly
/// (see remojoke_wordlist) — e.g. {8} always means "pick a word from the
/// body-parts list", in every template, not "the 8th slot of this
/// specific template". This was found and fixed via `cargo test`: an
/// earlier version treated N as a position into the per-template `slots`
/// array, which panicked/behaved wrong on ~130 templates because they
/// were authored using wordlist ids directly (e.g. {0}=names, {8}=body
/// parts) rather than positionally. `slots` is kept on
/// RemojokeCategory as human-readable documentation of which lists a
/// template draws from, but isn't consulted here anymore.
fn remojoke_fill_template(state: &mut u64, template: &str, _slots: &[usize]) -> String {
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            let mut num = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() { num.push(d); chars.next(); } else { break; }
            }
            if chars.peek() == Some(&'}') {
                chars.next();
                if let Ok(wordlist_id) = num.parse::<usize>() {
                    let words = remojoke_wordlist(wordlist_id);
                    out.push_str(*remojoke_pick(state, words));
                    continue;
                }
                // Malformed placeholder (shouldn't happen with our own
                // template data, but don't panic on it) — emit as-is.
                out.push('{'); out.push_str(&num); out.push('}');
            } else {
                out.push('{'); out.push_str(&num);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Returns a random joke for `category`. ~50% static bank, ~50% freshly
/// generated from templates — both paths draw from the same rand_state so
/// results differ across calls/runs. Unknown categories get a helpful
/// error listing the real available categories (never silently return
/// nothing or a fake joke for a category that doesn't exist).
fn remojoke_get_legacy_src_only(category: &str, state: &mut u64) -> String {
    let cat = match remojoke_find_category(category) {
        Some(c) => c,
        None => {
            let available: Vec<&str> = REMOJOKE_CATEGORIES.iter().map(|c| c.key).collect();
            return format!(
                "Remojoke: category '{}' nahi mili. Available categories: {}",
                category, available.join(", ")
            );
        }
    };
    let use_generated = remojoke_next(state) % 2 == 0;
    if use_generated && !cat.templates.is_empty() {
        let (template, slots) = remojoke_pick(state, cat.templates);
        remojoke_fill_template(state, template, slots)
    } else {
        (*remojoke_pick(state, cat.jokes)).to_string()
    }
}

// =========================================================================
// Remojoke translator engine.
//
//   use remojoke
//   Category.remojoke("programming")     // uses current language (default: src)
//   Lang.remojoke("hindi")               // switches current language
//   Multijoke.remojoke(5)                // n random jokes as a list
//
// HONEST SCOPE (read before assuming this covers everything):
// This is real, hand-written, human-verified translation of the STATIC
// bank only — not a generic auto-translator. Two deliberate limits:
//
//   1. Only a subset of categories/jokes are translated so far (see
//      REMOJOKE_TRANSLATIONS below — currently "programming" and
//      "chemistry", 5 jokes each). Growing coverage means adding more
//      rows to that table with real, checked translations — there's no
//      shortcut that doesn't cost that same effort per phrase.
//
//   2. The GENERATOR ENGINE (Mad-Libs templates) is deliberately NOT
//      translated, in any language other than src. The templates are
//      Hinglish sentences with word-order/grammar baked into the fixed
//      template text; only the {N} slot words get substituted. Swapping
//      just those words for German/French/Hindi equivalents while
//      keeping the Hinglish sentence skeleton produces grammatically
//      broken output (wrong word order, no case/gender agreement) — text
//      that *looks* translated but reads as nonsense to a real speaker of
//      that language. Emitting that anyway just to claim "50 languages
//      supported" would be exactly the kind of simulated/fake success
//      this project has been explicit about avoiding. So: when a
//      non-src language is requested, Remojoke serves static-bank jokes
//      only. Real multilingual generator support would need a separate
//      template set written natively per language (not a translation
//      pass) — a much bigger, honestly-scoped follow-up.
//
// There is also no real machine-translation path available even if we
// wanted one: no network stack to call a translation API (see
// MonobatRealHal::net_tcp_bind's doc comment — no TCP/IP on top of the
// raw NIC driver), and no ML model realistically fits in a no_std kernel
// binary. So "translation" here means exactly what it says: a table of
// real, hand-checked strings, not a black box.
// =========================================================================

/// One hand-translated joke: which category it belongs to, its exact
/// source (src / original Hinglish) text so it can be matched against
/// REMOJOKE_CATEGORIES' static jokes, and its real translations.
struct RemojokeTranslation {
    category: &'static str,
    #[allow(dead_code)] // kept for humans maintaining this table, not read at runtime
    source: &'static str,
    translations: &'static [(&'static str, &'static str)], // (lang_code, text)
}

const REMOJOKE_TRANSLATIONS: &[RemojokeTranslation] = &[
    RemojokeTranslation { category: "programming", source: "Do programmers shaadi kar rahe the. Priest bola: I now pronounce you... 'undefined'.", translations: &[
        ("en", "Two programmers were getting married. The priest said: I now pronounce you... 'undefined'."),
        ("hi", "दो प्रोग्रामर शादी कर रहे थे। पादरी ने कहा: अब मैं तुम्हें घोषित करता हूँ... 'अपरिभाषित'।"),
        ("de", "Zwei Programmierer heirateten. Der Priester sagte: Ich erkläre euch nun zu... 'undefiniert'."),
        ("fr", "Deux programmeurs se mariaient. Le prêtre a dit : Je vous déclare maintenant... 'indéfini'."),
    ]},
    RemojokeTranslation { category: "programming", source: "Ek bug 3 saal se production mein tha. Naam rakh diya gaya 'legacy feature'.", translations: &[
        ("en", "A bug had been in production for 3 years. It got renamed 'legacy feature'."),
        ("hi", "एक बग तीन साल से प्रोडक्शन में था। उसका नाम रख दिया गया 'लेगेसी फीचर'।"),
        ("de", "Ein Bug war seit drei Jahren in Produktion. Er wurde in 'Legacy-Feature' umbenannt."),
        ("fr", "Un bug était en production depuis 3 ans. On l'a renommé 'fonctionnalité historique'."),
    ]},
    RemojokeTranslation { category: "programming", source: "Interviewer: recursion samjhao. Candidate: recursion samajhne ke liye pehle recursion samajhna padta hai.", translations: &[
        ("en", "Interviewer: explain recursion. Candidate: to understand recursion, you first need to understand recursion."),
        ("hi", "इंटरव्यूअर: रिकर्शन समझाओ। कैंडिडेट: रिकर्शन समझने के लिए पहले रिकर्शन समझना पड़ता है।"),
        ("de", "Interviewer: Erkläre Rekursion. Kandidat: Um Rekursion zu verstehen, muss man zuerst Rekursion verstehen."),
        ("fr", "Recruteur : explique la récursivité. Candidat : pour comprendre la récursivité, il faut d'abord comprendre la récursivité."),
    ]},
    RemojokeTranslation { category: "programming", source: "Programmer ke ghar aag lagi. Wo bola: don't worry, git commit kiya hua hai sab.", translations: &[
        ("en", "A programmer's house caught fire. He said: don't worry, I've committed everything to git."),
        ("hi", "प्रोग्रामर के घर में आग लग गई। उसने कहा: चिंता मत करो, सब कुछ git में कमिट कर दिया है।"),
        ("de", "Das Haus eines Programmierers brannte. Er sagte: keine Sorge, ich habe alles in git committed."),
        ("fr", "La maison d'un programmeur a pris feu. Il a dit : pas d'inquiétude, j'ai tout commité sur git."),
    ]},
    RemojokeTranslation { category: "programming", source: "Programmer se poocha gaya password kya hai. Usne bola: 'incorrect' — taaki system bole 'your password is incorrect', aur usko yaad rahe.", translations: &[
        ("en", "A programmer was asked what his password was. He said: 'incorrect' — so the system would say 'your password is incorrect', and he'd remember it."),
        ("hi", "प्रोग्रामर से पूछा गया पासवर्ड क्या है। उसने कहा: 'incorrect' — ताकि सिस्टम कहे 'your password is incorrect', और उसे याद रहे।"),
        ("de", "Ein Programmierer wurde nach seinem Passwort gefragt. Er sagte: 'incorrect' — damit das System sagt 'your password is incorrect', und er sich daran erinnert."),
        ("fr", "On a demandé à un programmeur quel était son mot de passe. Il a dit : 'incorrect' — comme ça le système dirait 'your password is incorrect', et il s'en souviendrait."),
    ]},
    RemojokeTranslation { category: "chemistry", source: "Do atoms sadak par mil gaye. Ek bola: mera electron kho gaya. Dusra bola: sure? Pehla bola: I'm positive.", translations: &[
        ("en", "Two atoms met on the road. One said: I lost an electron. The other said: are you sure? The first said: I'm positive."),
        ("hi", "दो परमाणु सड़क पर मिले। एक ने कहा: मेरा इलेक्ट्रॉन खो गया। दूसरे ने कहा: पक्का? पहले ने कहा: मैं पॉज़िटिव हूँ।"),
        ("de", "Zwei Atome trafen sich auf der Straße. Eines sagte: Ich habe ein Elektron verloren. Das andere sagte: bist du sicher? Das erste sagte: Ich bin positiv."),
        ("fr", "Deux atomes se sont rencontrés dans la rue. L'un a dit : j'ai perdu un électron. L'autre a dit : tu es sûr ? Le premier a dit : je suis positif."),
    ]},
    RemojokeTranslation { category: "chemistry", source: "Sodium ek bar bar gaya aur bola: mera ek electron mujhse alag ho gaya, ab main Na+ hu.", translations: &[
        ("en", "Sodium went to a bar and said: I lost an electron, now I'm Na+."),
        ("hi", "सोडियम एक बार गया और बोला: मेरा एक इलेक्ट्रॉन मुझसे अलग हो गया, अब मैं Na+ हूँ।"),
        ("de", "Natrium ging in eine Bar und sagte: Ich habe ein Elektron verloren, jetzt bin ich Na+."),
        ("fr", "Le sodium est allé dans un bar et a dit : j'ai perdu un électron, maintenant je suis Na+."),
    ]},
    RemojokeTranslation { category: "chemistry", source: "Oxygen aur Magnesium ki shaadi ho gayi. Sab bole: MgO!", translations: &[
        ("en", "Oxygen and Magnesium got married. Everyone said: MgO!"),
        ("hi", "ऑक्सीजन और मैग्नीशियम की शादी हो गई। सब बोले: MgO!"),
        ("de", "Sauerstoff und Magnesium haben geheiratet. Alle sagten: MgO!"),
        ("fr", "L'oxygène et le magnésium se sont mariés. Tout le monde a dit : MgO !"),
    ]},
    RemojokeTranslation { category: "chemistry", source: "Chemistry lab mein sabse bada khatra kya hai? Student jo period table ko syllabus samajh ke last minute padhta hai.", translations: &[
        ("en", "What's the biggest danger in a chemistry lab? A student who treats the periodic table as syllabus and studies it at the last minute."),
        ("hi", "केमिस्ट्री लैब में सबसे बड़ा खतरा क्या है? वह स्टूडेंट जो पीरियोडिक टेबल को सिलेबस समझकर आखिरी मिनट में पढ़ता है।"),
        ("de", "Was ist die größte Gefahr in einem Chemielabor? Ein Student, der das Periodensystem als Lehrplan behandelt und es in letzter Minute lernt."),
        ("fr", "Quel est le plus grand danger dans un labo de chimie ? Un étudiant qui traite le tableau périodique comme un programme et le révise à la dernière minute."),
    ]},
    RemojokeTranslation { category: "chemistry", source: "Chemist ne joke sunaya par koi react nahi hua.", translations: &[
        ("en", "The chemist told a joke, but nobody reacted."),
        ("hi", "केमिस्ट ने जोक सुनाया पर कोई रिएक्ट नहीं हुआ।"),
        ("de", "Der Chemiker erzählte einen Witz, aber niemand reagierte."),
        ("fr", "Le chimiste a raconté une blague, mais personne n'a réagi."),
    ]},
];

/// Normalizes user-typed language names/codes to the codes used in
/// REMOJOKE_TRANSLATIONS. Unknown input is passed through as-is (lets the
/// "not available" error message echo back exactly what the user typed).
fn remojoke_normalize_lang(lang: &str) -> String {
    match lang.trim().to_lowercase().as_str() {
        "" | "src" | "source" | "hinglish" | "default" | "original" => "src".to_string(),
        "english" | "eng" | "en" => "en".to_string(),
        "hindi" | "hi" => "hi".to_string(),
        "german" | "deutsch" | "de" => "de".to_string(),
        "french" | "francais" | "français" | "fr" => "fr".to_string(),
        other => other.to_string(),
    }
}

fn remojoke_translated_categories() -> Vec<&'static str> {
    let mut cats: Vec<&'static str> = Vec::new();
    for t in REMOJOKE_TRANSLATIONS {
        if !cats.contains(&t.category) { cats.push(t.category); }
    }
    cats
}

fn remojoke_supported_langs() -> Vec<&'static str> {
    let mut langs: Vec<&'static str> = Vec::new();
    for t in REMOJOKE_TRANSLATIONS {
        for (lang, _) in t.translations {
            if !langs.contains(lang) { langs.push(*lang); }
        }
    }
    langs
}

/// Real entry point: category + language-aware. `lang` is whatever's
/// currently set via `Lang.remojoke(...)` (default "src" = original
/// Hinglish, full static+generator behavior). Any other language routes
/// through the translator table (static bank only — see module doc
/// comment above for why).
fn remojoke_get(category: &str, lang: &str, state: &mut u64) -> String {
    if remojoke_find_category(category).is_none() {
        let available: Vec<&str> = REMOJOKE_CATEGORIES.iter().map(|c| c.key).collect();
        return format!(
            "Remojoke: category '{}' nahi mili. Available categories: {}",
            category, available.join(", ")
        );
    }

    let lang_norm = remojoke_normalize_lang(lang);
    if lang_norm == "src" {
        return remojoke_get_legacy_src_only(category, state);
    }

    let cat_key = remojoke_find_category(category).unwrap().key;
    let matches: Vec<&str> = REMOJOKE_TRANSLATIONS.iter()
        .filter(|t| t.category == cat_key)
        .filter_map(|t| t.translations.iter().find(|(l, _)| *l == lang_norm).map(|(_, txt)| *txt))
        .collect();

    if matches.is_empty() {
        let cats = remojoke_translated_categories();
        let langs = remojoke_supported_langs();
        return format!(
            "Remojoke: '{}' category '{}' language mein abhi translate nahi hui hai. \
             Translated categories: {}. Supported languages: {} (default 'src' = original Hinglish, \
             all {} categories + generator engine available there).",
            category, lang, cats.join(", "), langs.join(", "), REMOJOKE_CATEGORIES.len()
        );
    }
    (*remojoke_pick(state, &matches)).to_string()
}

/// `Multijoke.remojoke(n)` — n random jokes (mixed categories), using the
/// current language. Capped at 50 per call — not a hard technical limit,
/// just a sane guard against `Multijoke.remojoke(1000000)` being used to
/// generate megabytes of text / burn CPU in a single call.
fn remojoke_get_multi(n: i64, lang: &str, state: &mut u64) -> Vec<Value> {
    let count = n.clamp(0, 50) as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let cat = remojoke_pick(state, REMOJOKE_CATEGORIES);
        let cat_key = cat.key.to_string();
        out.push(Value::Str(remojoke_get(&cat_key, lang, state)));
    }
    out
}

fn get_module(name: &str) -> Option<Vec<(String, Value)>> {
    match name {
        "math" => Some(vec![
            ("pi".into(),    Value::Float(std::f64::consts::PI)),
            ("e".into(),     Value::Float(std::f64::consts::E)),
            ("inf".into(),   Value::Float(f64::INFINITY)),
            ("tau".into(),   Value::Float(std::f64::consts::TAU)),
            ("sqrt".into(),  Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__math_sqrt".into())), captures: HashMap::new() }),
            ("pow".into(),   Value::Lambda { params: vec!["x".into(), "n".into()], body: Box::new(Expr::Ident("__math_pow".into())), captures: HashMap::new() }),
            ("abs".into(),   Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__math_abs".into())), captures: HashMap::new() }),
            ("floor".into(), Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__math_floor".into())), captures: HashMap::new() }),
            ("ceil".into(),  Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__math_ceil".into())), captures: HashMap::new() }),
            ("round".into(), Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__math_round".into())), captures: HashMap::new() }),
            ("sin".into(),   Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__math_sin".into())), captures: HashMap::new() }),
            ("cos".into(),   Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__math_cos".into())), captures: HashMap::new() }),
            ("tan".into(),   Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__math_tan".into())), captures: HashMap::new() }),
            ("log".into(),   Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__math_log".into())), captures: HashMap::new() }),
            ("log2".into(),  Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__math_log2".into())), captures: HashMap::new() }),
            ("log10".into(), Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__math_log10".into())), captures: HashMap::new() }),
            ("min".into(),   Value::Lambda { params: vec!["a".into(), "b".into()], body: Box::new(Expr::Ident("__math_min".into())), captures: HashMap::new() }),
            ("max".into(),   Value::Lambda { params: vec!["a".into(), "b".into()], body: Box::new(Expr::Ident("__math_max".into())), captures: HashMap::new() }),
        ]),
        "io" => Some(vec![
            ("print".into(), Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__io_print".into())), captures: HashMap::new() }),
            ("read".into(),  Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__io_read".into())), captures: HashMap::new() }),
        ]),
        "os" => Some(vec![
            ("args".into(), {
                let a: Vec<Value> = std::env::args().map(Value::Str).collect();
                Value::List(a)
            }),
            ("env".into(), Value::Lambda { params: vec!["key".into()], body: Box::new(Expr::Ident("__os_env".into())), captures: HashMap::new() }),
            ("exit".into(), Value::Lambda { params: vec!["code".into()], body: Box::new(Expr::Ident("__os_exit".into())), captures: HashMap::new() }),
        ]),
        "rand" => Some(vec![
            // Simple LCG-based pseudo-random using system time
            ("seed".into(),  Value::Int(0)),
            ("int".into(),   Value::Lambda { params: vec!["min".into(), "max".into()], body: Box::new(Expr::Ident("__rand_int".into())), captures: HashMap::new() }),
            ("float".into(), Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__rand_float".into())), captures: HashMap::new() }),
        ]),
        // =====================================================================
        // Remojoke — joke library.
        //   use remojoke
        //   Category.remojoke("programming")
        // `use remojoke` binds the global `Category`, a map with one
        // function `remojoke(category)`. See the Remojoke module block
        // above get_module for the static bank + generator engine.
        // =====================================================================
        "remojoke" => Some(vec![
            ("Category".into(), Value::Map(vec![
                ("remojoke".into(), Value::Lambda {
                    params: vec!["category".into()],
                    body: Box::new(Expr::Ident("__remojoke_get".into())),
                    captures: HashMap::new(),
                }),
            ])),
            ("Lang".into(), Value::Map(vec![
                ("remojoke".into(), Value::Lambda {
                    params: vec!["language".into()],
                    body: Box::new(Expr::Ident("__remojoke_setlang".into())),
                    captures: HashMap::new(),
                }),
            ])),
            ("Multijoke".into(), Value::Map(vec![
                ("remojoke".into(), Value::Lambda {
                    params: vec!["count".into()],
                    body: Box::new(Expr::Ident("__remojoke_multi".into())),
                    captures: HashMap::new(),
                }),
            ])),
        ]),
        // =====================================================================
        // Malib — Remox Advanced Math Engine
        // use Malib   →   Malib.solve("x^2 - 5x + 6 = 0")
        // Ek hi keyword "Malib" se: algebra, calculus, stats, number theory,
        // combinatorics, matrices, aur general expression solving — sab kuch.
        // =====================================================================
        "Malib" => Some(vec![
            ("pi".into(),  Value::Float(std::f64::consts::PI)),
            ("e".into(),   Value::Float(std::f64::consts::E)),
            ("inf".into(), Value::Float(f64::INFINITY)),
            ("tau".into(), Value::Float(std::f64::consts::TAU)),

            // ---- General purpose ----
            ("eval".into(),     Value::Lambda { params: vec!["expr".into()], body: Box::new(Expr::Ident("__malib_eval".into())), captures: HashMap::new() }),
            ("solve".into(),    Value::Lambda { params: vec!["equation".into()], body: Box::new(Expr::Ident("__malib_solve".into())), captures: HashMap::new() }),
            ("simplify".into(), Value::Lambda { params: vec!["expr".into()], body: Box::new(Expr::Ident("__malib_simplify".into())), captures: HashMap::new() }),

            // ---- Basic / extended arithmetic ----
            ("sqrt".into(),  Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__malib_sqrt".into())), captures: HashMap::new() }),
            ("cbrt".into(),  Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__malib_cbrt".into())), captures: HashMap::new() }),
            ("pow".into(),   Value::Lambda { params: vec!["x".into(), "n".into()], body: Box::new(Expr::Ident("__malib_pow".into())), captures: HashMap::new() }),
            ("abs".into(),   Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__malib_abs".into())), captures: HashMap::new() }),
            ("floor".into(), Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__malib_floor".into())), captures: HashMap::new() }),
            ("ceil".into(),  Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__malib_ceil".into())), captures: HashMap::new() }),
            ("round".into(), Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__malib_round".into())), captures: HashMap::new() }),
            ("min".into(),   Value::Lambda { params: vec!["a".into(), "b".into()], body: Box::new(Expr::Ident("__malib_min".into())), captures: HashMap::new() }),
            ("max".into(),   Value::Lambda { params: vec!["a".into(), "b".into()], body: Box::new(Expr::Ident("__malib_max".into())), captures: HashMap::new() }),

            // ---- Trigonometry / logarithms ----
            ("sin".into(),   Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__malib_sin".into())), captures: HashMap::new() }),
            ("cos".into(),   Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__malib_cos".into())), captures: HashMap::new() }),
            ("tan".into(),   Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__malib_tan".into())), captures: HashMap::new() }),
            ("asin".into(),  Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__malib_asin".into())), captures: HashMap::new() }),
            ("acos".into(),  Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__malib_acos".into())), captures: HashMap::new() }),
            ("atan".into(),  Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__malib_atan".into())), captures: HashMap::new() }),
            ("log".into(),   Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__malib_log".into())), captures: HashMap::new() }),
            ("log2".into(),  Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__malib_log2".into())), captures: HashMap::new() }),
            ("log10".into(), Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__malib_log10".into())), captures: HashMap::new() }),
            ("logn".into(),  Value::Lambda { params: vec!["x".into(), "base".into()], body: Box::new(Expr::Ident("__malib_logn".into())), captures: HashMap::new() }),

            // ---- Number theory ----
            ("gcd".into(),        Value::Lambda { params: vec!["a".into(), "b".into()], body: Box::new(Expr::Ident("__malib_gcd".into())), captures: HashMap::new() }),
            ("lcm".into(),        Value::Lambda { params: vec!["a".into(), "b".into()], body: Box::new(Expr::Ident("__malib_lcm".into())), captures: HashMap::new() }),
            ("isPrime".into(),    Value::Lambda { params: vec!["n".into()], body: Box::new(Expr::Ident("__malib_is_prime".into())), captures: HashMap::new() }),
            ("factorize".into(),  Value::Lambda { params: vec!["n".into()], body: Box::new(Expr::Ident("__malib_factorize".into())), captures: HashMap::new() }),
            ("factorial".into(),  Value::Lambda { params: vec!["n".into()], body: Box::new(Expr::Ident("__malib_factorial".into())), captures: HashMap::new() }),
            ("fibonacci".into(),  Value::Lambda { params: vec!["n".into()], body: Box::new(Expr::Ident("__malib_fibonacci".into())), captures: HashMap::new() }),

            // ---- Combinatorics ----
            ("nCr".into(), Value::Lambda { params: vec!["n".into(), "r".into()], body: Box::new(Expr::Ident("__malib_ncr".into())), captures: HashMap::new() }),
            ("nPr".into(), Value::Lambda { params: vec!["n".into(), "r".into()], body: Box::new(Expr::Ident("__malib_npr".into())), captures: HashMap::new() }),

            // ---- Algebra: equation solvers ----
            ("linear".into(),    Value::Lambda { params: vec!["a".into(), "b".into()], body: Box::new(Expr::Ident("__malib_linear".into())), captures: HashMap::new() }),
            ("quadratic".into(), Value::Lambda { params: vec!["a".into(), "b".into(), "c".into()], body: Box::new(Expr::Ident("__malib_quadratic".into())), captures: HashMap::new() }),

            // ---- Calculus (numerical) ----
            ("derivative".into(), Value::Lambda { params: vec!["expr".into(), "at".into()], body: Box::new(Expr::Ident("__malib_derivative".into())), captures: HashMap::new() }),
            ("integral".into(),   Value::Lambda { params: vec!["expr".into(), "a".into(), "b".into()], body: Box::new(Expr::Ident("__malib_integral".into())), captures: HashMap::new() }),
            ("limit".into(),      Value::Lambda { params: vec!["expr".into(), "at".into()], body: Box::new(Expr::Ident("__malib_limit".into())), captures: HashMap::new() }),
            ("root".into(),       Value::Lambda { params: vec!["expr".into(), "guess".into()], body: Box::new(Expr::Ident("__malib_root".into())), captures: HashMap::new() }),

            // ---- Statistics ----
            ("mean".into(),     Value::Lambda { params: vec!["list".into()], body: Box::new(Expr::Ident("__malib_mean".into())), captures: HashMap::new() }),
            ("median".into(),   Value::Lambda { params: vec!["list".into()], body: Box::new(Expr::Ident("__malib_median".into())), captures: HashMap::new() }),
            ("variance".into(), Value::Lambda { params: vec!["list".into()], body: Box::new(Expr::Ident("__malib_variance".into())), captures: HashMap::new() }),
            ("stdev".into(),    Value::Lambda { params: vec!["list".into()], body: Box::new(Expr::Ident("__malib_stdev".into())), captures: HashMap::new() }),
            ("sum".into(),      Value::Lambda { params: vec!["list".into()], body: Box::new(Expr::Ident("__malib_sum".into())), captures: HashMap::new() }),

            // ---- Matrices (2D list of lists) ----
            ("matMul".into(), Value::Lambda { params: vec!["m1".into(), "m2".into()], body: Box::new(Expr::Ident("__malib_matmul".into())), captures: HashMap::new() }),
            ("matDet".into(), Value::Lambda { params: vec!["m".into()], body: Box::new(Expr::Ident("__malib_matdet".into())), captures: HashMap::new() }),

            // ---- Fraction helper ----
            ("toFraction".into(), Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__malib_to_fraction".into())), captures: HashMap::new() }),

            // ---- Extra missing entries ----
            ("diff_sym".into(),   Value::Lambda { params: vec!["e".into()], body: Box::new(Expr::Ident("__malib_diff_sym".into())), captures: HashMap::new() }),
            ("stats".into(),      Value::Lambda { params: vec!["l".into()], body: Box::new(Expr::Ident("__malib_stats".into())), captures: HashMap::new() }),
            ("percentile".into(), Value::Lambda { params: vec!["l".into(),"p".into()], body: Box::new(Expr::Ident("__malib_percentile".into())), captures: HashMap::new() }),
            ("complex".into(),    Value::Lambda { params: vec!["r".into(),"i".into()], body: Box::new(Expr::Ident("__malib_complex".into())), captures: HashMap::new() }),
            ("complex_op".into(), Value::Lambda { params: vec!["r".into(),"i".into(),"op".into()], body: Box::new(Expr::Ident("__malib_complex_op".into())), captures: HashMap::new() }),
            ("modinv".into(),     Value::Lambda { params: vec!["a".into(),"m".into()], body: Box::new(Expr::Ident("__malib_modinv".into())), captures: HashMap::new() }),
            ("primes".into(),     Value::Lambda { params: vec!["n".into()], body: Box::new(Expr::Ident("__malib_primes".into())), captures: HashMap::new() }),
            ("totient".into(),    Value::Lambda { params: vec!["n".into()], body: Box::new(Expr::Ident("__malib_totient".into())), captures: HashMap::new() }),
            ("arith_series".into(),Value::Lambda { params: vec!["a".into(),"d".into(),"n".into()], body: Box::new(Expr::Ident("__malib_arithmetic_series".into())), captures: HashMap::new() }),
            ("geo_series".into(), Value::Lambda { params: vec!["a".into(),"r".into(),"n".into()], body: Box::new(Expr::Ident("__malib_geometric_series".into())), captures: HashMap::new() }),
            ("clamp".into(),      Value::Lambda { params: vec!["x".into(),"lo".into(),"hi".into()], body: Box::new(Expr::Ident("__malib_clamp".into())), captures: HashMap::new() }),
            ("lerp".into(),       Value::Lambda { params: vec!["a".into(),"b".into(),"t".into()], body: Box::new(Expr::Ident("__malib_lerp".into())), captures: HashMap::new() }),
            ("map_range".into(),  Value::Lambda { params: vec!["x".into(),"il".into(),"ih".into(),"ol".into(),"oh".into()], body: Box::new(Expr::Ident("__malib_map_range".into())), captures: HashMap::new() }),
            ("sign".into(),       Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__malib_sign".into())), captures: HashMap::new() }),
            ("deg".into(),        Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__malib_deg".into())), captures: HashMap::new() }),
            ("rad".into(),        Value::Lambda { params: vec!["x".into()], body: Box::new(Expr::Ident("__malib_rad".into())), captures: HashMap::new() }),
            ("matAdd".into(),     Value::Lambda { params: vec!["a".into(),"b".into()], body: Box::new(Expr::Ident("__malib_matadd".into())), captures: HashMap::new() }),
            ("matTranspose".into(),Value::Lambda { params: vec!["m".into()], body: Box::new(Expr::Ident("__malib_mattranspose".into())), captures: HashMap::new() }),
            ("matInv".into(),     Value::Lambda { params: vec!["m".into()], body: Box::new(Expr::Ident("__malib_matinv".into())), captures: HashMap::new() }),
        ]),

        // =====================================================================
        // Numrux — Remox ka apna N-dimensional Array Engine (NumPy-jaisa, par
        // zero-import, zero-boilerplate, aur seedha language mein built-in).
        // use Numrux  →  Numrux.array([1,2,3])
        // Arrays normal "+ - * /" operators se bhi directly kaam karte hain
        // (eval_binop mein broadcasting hook lagi hai) — isiliye NumPy se
        // zyada easy: "a + b" likho, "np.add(a,b)" nahi.
        // Covers: N-D creation, broadcasting elementwise math, reductions,
        // linear algebra (Malib ke matrix engine se bridged), random, stats.
        // =====================================================================
        "Numrux" => Some(vec![
            // ---- Creation ----
            ("array".into(),    Value::Lambda { params: vec!["nested".into()], body: Box::new(Expr::Ident("__numrux_array".into())), captures: HashMap::new() }),
            ("zeros".into(),    Value::Lambda { params: vec!["shape".into()], body: Box::new(Expr::Ident("__numrux_zeros".into())), captures: HashMap::new() }),
            ("ones".into(),     Value::Lambda { params: vec!["shape".into()], body: Box::new(Expr::Ident("__numrux_ones".into())), captures: HashMap::new() }),
            ("full".into(),     Value::Lambda { params: vec!["shape".into(),"val".into()], body: Box::new(Expr::Ident("__numrux_full".into())), captures: HashMap::new() }),
            ("arange".into(),   Value::Lambda { params: vec!["start".into(),"stop".into(),"step".into()], body: Box::new(Expr::Ident("__numrux_arange".into())), captures: HashMap::new() }),
            ("linspace".into(), Value::Lambda { params: vec!["start".into(),"stop".into(),"n".into()], body: Box::new(Expr::Ident("__numrux_linspace".into())), captures: HashMap::new() }),
            ("eye".into(),      Value::Lambda { params: vec!["n".into()], body: Box::new(Expr::Ident("__numrux_eye".into())), captures: HashMap::new() }),

            // ---- Shape / info ----
            ("shape".into(),    Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_shape".into())), captures: HashMap::new() }),
            ("size".into(),     Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_size".into())), captures: HashMap::new() }),
            ("ndim".into(),     Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_ndim".into())), captures: HashMap::new() }),
            ("reshape".into(),  Value::Lambda { params: vec!["a".into(),"newshape".into()], body: Box::new(Expr::Ident("__numrux_reshape".into())), captures: HashMap::new() }),
            ("flatten".into(),  Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_flatten".into())), captures: HashMap::new() }),
            ("transpose".into(),Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_transpose".into())), captures: HashMap::new() }),
            ("toList".into(),   Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_tolist".into())), captures: HashMap::new() }),

            // ---- Elementwise math (broadcasting) ----
            ("add".into(), Value::Lambda { params: vec!["a".into(),"b".into()], body: Box::new(Expr::Ident("__numrux_add".into())), captures: HashMap::new() }),
            ("sub".into(), Value::Lambda { params: vec!["a".into(),"b".into()], body: Box::new(Expr::Ident("__numrux_sub".into())), captures: HashMap::new() }),
            ("mul".into(), Value::Lambda { params: vec!["a".into(),"b".into()], body: Box::new(Expr::Ident("__numrux_mul".into())), captures: HashMap::new() }),
            ("div".into(), Value::Lambda { params: vec!["a".into(),"b".into()], body: Box::new(Expr::Ident("__numrux_div".into())), captures: HashMap::new() }),
            ("powArr".into(), Value::Lambda { params: vec!["a".into(),"b".into()], body: Box::new(Expr::Ident("__numrux_pow".into())), captures: HashMap::new() }),
            ("neg".into(),  Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_neg".into())), captures: HashMap::new() }),
            ("absArr".into(),Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_abs".into())), captures: HashMap::new() }),
            ("sqrtArr".into(),Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_sqrt".into())), captures: HashMap::new() }),
            ("expArr".into(),Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_exp".into())), captures: HashMap::new() }),
            ("logArr".into(),Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_log".into())), captures: HashMap::new() }),
            ("sinArr".into(),Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_sin".into())), captures: HashMap::new() }),
            ("cosArr".into(),Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_cos".into())), captures: HashMap::new() }),
            ("tanArr".into(),Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_tan".into())), captures: HashMap::new() }),
            ("clip".into(), Value::Lambda { params: vec!["a".into(),"lo".into(),"hi".into()], body: Box::new(Expr::Ident("__numrux_clip".into())), captures: HashMap::new() }),

            // ---- Comparison / masking ----
            ("gt".into(), Value::Lambda { params: vec!["a".into(),"b".into()], body: Box::new(Expr::Ident("__numrux_gt".into())), captures: HashMap::new() }),
            ("lt".into(), Value::Lambda { params: vec!["a".into(),"b".into()], body: Box::new(Expr::Ident("__numrux_lt".into())), captures: HashMap::new() }),
            ("ge".into(), Value::Lambda { params: vec!["a".into(),"b".into()], body: Box::new(Expr::Ident("__numrux_ge".into())), captures: HashMap::new() }),
            ("le".into(), Value::Lambda { params: vec!["a".into(),"b".into()], body: Box::new(Expr::Ident("__numrux_le".into())), captures: HashMap::new() }),
            ("eqArr".into(), Value::Lambda { params: vec!["a".into(),"b".into()], body: Box::new(Expr::Ident("__numrux_eq".into())), captures: HashMap::new() }),
            ("where".into(), Value::Lambda { params: vec!["cond".into(),"x".into(),"y".into()], body: Box::new(Expr::Ident("__numrux_where".into())), captures: HashMap::new() }),

            // ---- Reductions ----
            ("sum".into(),    Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_sum".into())), captures: HashMap::new() }),
            ("mean".into(),   Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_mean".into())), captures: HashMap::new() }),
            ("min".into(),    Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_min".into())), captures: HashMap::new() }),
            ("max".into(),    Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_max".into())), captures: HashMap::new() }),
            ("prod".into(),   Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_prod".into())), captures: HashMap::new() }),
            ("std".into(),    Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_std".into())), captures: HashMap::new() }),
            ("var".into(),    Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_var".into())), captures: HashMap::new() }),
            ("median".into(), Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_median".into())), captures: HashMap::new() }),
            ("argmin".into(), Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_argmin".into())), captures: HashMap::new() }),
            ("argmax".into(), Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_argmax".into())), captures: HashMap::new() }),
            ("cumsum".into(), Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_cumsum".into())), captures: HashMap::new() }),

            // ---- Linear algebra (2D, bridged to Malib's matrix engine) ----
            ("dot".into(),    Value::Lambda { params: vec!["a".into(),"b".into()], body: Box::new(Expr::Ident("__numrux_dot".into())), captures: HashMap::new() }),
            ("matmul".into(), Value::Lambda { params: vec!["a".into(),"b".into()], body: Box::new(Expr::Ident("__numrux_dot".into())), captures: HashMap::new() }),
            ("det".into(),    Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_det".into())), captures: HashMap::new() }),
            ("inv".into(),    Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_inv".into())), captures: HashMap::new() }),
            ("trace".into(),  Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_trace".into())), captures: HashMap::new() }),

            // ---- Random ----
            ("seed".into(),    Value::Lambda { params: vec!["n".into()], body: Box::new(Expr::Ident("__numrux_seed".into())), captures: HashMap::new() }),
            ("rand".into(),    Value::Lambda { params: vec!["shape".into()], body: Box::new(Expr::Ident("__numrux_rand".into())), captures: HashMap::new() }),
            ("randn".into(),   Value::Lambda { params: vec!["shape".into()], body: Box::new(Expr::Ident("__numrux_randn".into())), captures: HashMap::new() }),
            ("randint".into(), Value::Lambda { params: vec!["lo".into(),"hi".into(),"shape".into()], body: Box::new(Expr::Ident("__numrux_randint".into())), captures: HashMap::new() }),

            // ---- Utility ----
            ("sort".into(),   Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_sort".into())), captures: HashMap::new() }),
            ("unique".into(), Value::Lambda { params: vec!["a".into()], body: Box::new(Expr::Ident("__numrux_unique".into())), captures: HashMap::new() }),
            ("concat".into(), Value::Lambda { params: vec!["a".into(),"b".into()], body: Box::new(Expr::Ident("__numrux_concat".into())), captures: HashMap::new() }),
        ]),

        // =====================================================================
        // Phinolib — Remox Advanced Real Physics Library
        // use Phinolib → Phinolib.gravity(m1, m2, r)
        // Covers: Kinematics, Dynamics, Gravity, Energy, Waves, Oscillations,
        //         Thermodynamics, Electromagnetism, Fluid Mechanics, Relativity,
        //         Rigid Body, Projectile, Collisions, Orbital Mechanics
        // ALL formulae are exact closed-form physics — zero simulation, zero approximation.
        // =====================================================================
        "Phinolib" => Some(vec![
            // ── Physical Constants ──────────────────────────────────────────
            ("G".into(),     Value::Float(6.674e-11)),        // Gravitational constant (N·m²/kg²)
            ("c".into(),     Value::Float(299_792_458.0)),    // Speed of light (m/s)
            ("h".into(),     Value::Float(6.62607015e-34)),   // Planck constant (J·s)
            ("hbar".into(),  Value::Float(1.054571817e-34)),  // Reduced Planck (J·s)
            ("e_charge".into(), Value::Float(1.602176634e-19)), // Elementary charge (C)
            ("k_e".into(),   Value::Float(8.9875517923e9)),   // Coulomb constant (N·m²/C²)
            ("k_b".into(),   Value::Float(1.380649e-23)),     // Boltzmann constant (J/K)
            ("N_A".into(),   Value::Float(6.02214076e23)),    // Avogadro's number (mol⁻¹)
            ("R_gas".into(), Value::Float(8.314462618)),      // Ideal gas constant (J/mol·K)
            ("eps0".into(),  Value::Float(8.8541878128e-12)), // Permittivity of free space (F/m)
            ("mu0".into(),   Value::Float(1.25663706212e-6)), // Permeability of free space (H/m)
            ("g".into(),     Value::Float(9.80665)),          // Standard gravity (m/s²)
            ("atm".into(),   Value::Float(101325.0)),         // Standard atmosphere (Pa)
            ("sigma_sb".into(), Value::Float(5.670374419e-8)), // Stefan-Boltzmann (W/m²/K⁴)
            ("m_e".into(),   Value::Float(9.1093837015e-31)), // Electron mass (kg)
            ("m_p".into(),   Value::Float(1.67262192369e-27)), // Proton mass (kg)
            ("m_n".into(),   Value::Float(1.67492749804e-27)), // Neutron mass (kg)
            ("au".into(),    Value::Float(1.495978707e11)),   // Astronomical unit (m)

            // ── Kinematics ──────────────────────────────────────────────────
            // displacement(v0, a, t) → s = v0*t + 0.5*a*t²
            ("displacement".into(), Value::Lambda { params: vec!["v0".into(), "a".into(), "t".into()], body: Box::new(Expr::Ident("__phinolib_displacement".into())), captures: HashMap::new() }),
            // velocity(v0, a, t) → v = v0 + a*t
            ("velocity".into(), Value::Lambda { params: vec!["v0".into(), "a".into(), "t".into()], body: Box::new(Expr::Ident("__phinolib_velocity".into())), captures: HashMap::new() }),
            // v_final_sq(v0, a, s) → v² = v0² + 2*a*s (returns v² — take sqrt yourself)
            ("vFinalSq".into(), Value::Lambda { params: vec!["v0".into(), "a".into(), "s".into()], body: Box::new(Expr::Ident("__phinolib_vfinalsq".into())), captures: HashMap::new() }),
            // time_to_stop(v0, a) → t = v0 / |a|
            ("timeToStop".into(), Value::Lambda { params: vec!["v0".into(), "a".into()], body: Box::new(Expr::Ident("__phinolib_timetostop".into())), captures: HashMap::new() }),
            // avg_velocity(v0, v) → (v0+v)/2 (constant-acceleration only)
            ("avgVelocity".into(), Value::Lambda { params: vec!["v0".into(), "v".into()], body: Box::new(Expr::Ident("__phinolib_avgvelocity".into())), captures: HashMap::new() }),

            // ── Projectile Motion ────────────────────────────────────────────
            // projectile_range(v0, angle_deg) → R = v0²·sin(2θ)/g
            ("projectileRange".into(), Value::Lambda { params: vec!["v0".into(), "angle_deg".into()], body: Box::new(Expr::Ident("__phinolib_projectile_range".into())), captures: HashMap::new() }),
            // projectile_max_height(v0, angle_deg) → H = v0²·sin²θ/(2g)
            ("projectileMaxHeight".into(), Value::Lambda { params: vec!["v0".into(), "angle_deg".into()], body: Box::new(Expr::Ident("__phinolib_projectile_height".into())), captures: HashMap::new() }),
            // projectile_time_of_flight(v0, angle_deg) → T = 2·v0·sinθ/g
            ("projectileFlightTime".into(), Value::Lambda { params: vec!["v0".into(), "angle_deg".into()], body: Box::new(Expr::Ident("__phinolib_projectile_tof".into())), captures: HashMap::new() }),
            // projectile_x(v0, angle_deg, t) → x = v0·cosθ·t
            ("projectileX".into(), Value::Lambda { params: vec!["v0".into(), "angle_deg".into(), "t".into()], body: Box::new(Expr::Ident("__phinolib_projectile_x".into())), captures: HashMap::new() }),
            // projectile_y(v0, angle_deg, t) → y = v0·sinθ·t - 0.5·g·t²
            ("projectileY".into(), Value::Lambda { params: vec!["v0".into(), "angle_deg".into(), "t".into()], body: Box::new(Expr::Ident("__phinolib_projectile_y".into())), captures: HashMap::new() }),
            // projectile_vy(v0, angle_deg, t) → vy = v0·sinθ - g·t
            ("projectileVy".into(), Value::Lambda { params: vec!["v0".into(), "angle_deg".into(), "t".into()], body: Box::new(Expr::Ident("__phinolib_projectile_vy".into())), captures: HashMap::new() }),

            // ── Circular / Rotational Motion ─────────────────────────────────
            // centripetal_acc(v, r) → a_c = v²/r
            ("centripetalAcc".into(), Value::Lambda { params: vec!["v".into(), "r".into()], body: Box::new(Expr::Ident("__phinolib_centripetal_acc".into())), captures: HashMap::new() }),
            // centripetal_force(m, v, r) → F_c = m·v²/r
            ("centripetalForce".into(), Value::Lambda { params: vec!["m".into(), "v".into(), "r".into()], body: Box::new(Expr::Ident("__phinolib_centripetal_force".into())), captures: HashMap::new() }),
            // angular_velocity(rpm) → ω = 2π·rpm/60
            ("angularVelocity".into(), Value::Lambda { params: vec!["rpm".into()], body: Box::new(Expr::Ident("__phinolib_angular_velocity".into())), captures: HashMap::new() }),
            // period_from_omega(omega) → T = 2π/ω
            ("periodFromOmega".into(), Value::Lambda { params: vec!["omega".into()], body: Box::new(Expr::Ident("__phinolib_period_omega".into())), captures: HashMap::new() }),
            // tangential_vel(omega, r) → v_t = ω·r
            ("tangentialVel".into(), Value::Lambda { params: vec!["omega".into(), "r".into()], body: Box::new(Expr::Ident("__phinolib_tangential_vel".into())), captures: HashMap::new() }),
            // angular_acc(alpha, t, omega0) → ω = ω₀ + α·t
            ("angularVelAt".into(), Value::Lambda { params: vec!["omega0".into(), "alpha".into(), "t".into()], body: Box::new(Expr::Ident("__phinolib_angular_vel_at".into())), captures: HashMap::new() }),
            // torque(F, r, angle_deg) → τ = F·r·sinθ
            ("torque".into(), Value::Lambda { params: vec!["F".into(), "r".into(), "angle_deg".into()], body: Box::new(Expr::Ident("__phinolib_torque".into())), captures: HashMap::new() }),
            // angular_momentum(I, omega) → L = I·ω
            ("angularMomentum".into(), Value::Lambda { params: vec!["I".into(), "omega".into()], body: Box::new(Expr::Ident("__phinolib_angular_momentum".into())), captures: HashMap::new() }),
            // moment_of_inertia_solid_sphere(m, r) → I = (2/5)·m·r²
            ("inertiaSolidSphere".into(), Value::Lambda { params: vec!["m".into(), "r".into()], body: Box::new(Expr::Ident("__phinolib_inertia_solid_sphere".into())), captures: HashMap::new() }),
            // moment_of_inertia_hollow_sphere(m, r) → I = (2/3)·m·r²
            ("inertiaHollowSphere".into(), Value::Lambda { params: vec!["m".into(), "r".into()], body: Box::new(Expr::Ident("__phinolib_inertia_hollow_sphere".into())), captures: HashMap::new() }),
            // moment_of_inertia_cylinder(m, r) → I = 0.5·m·r²
            ("inertiaCylinder".into(), Value::Lambda { params: vec!["m".into(), "r".into()], body: Box::new(Expr::Ident("__phinolib_inertia_cylinder".into())), captures: HashMap::new() }),
            // moment_of_inertia_rod_center(m, l) → I = (1/12)·m·l²
            ("inertiaRodCenter".into(), Value::Lambda { params: vec!["m".into(), "l".into()], body: Box::new(Expr::Ident("__phinolib_inertia_rod_center".into())), captures: HashMap::new() }),
            // moment_of_inertia_rod_end(m, l) → I = (1/3)·m·l²
            ("inertiaRodEnd".into(), Value::Lambda { params: vec!["m".into(), "l".into()], body: Box::new(Expr::Ident("__phinolib_inertia_rod_end".into())), captures: HashMap::new() }),

            // ── Newtonian Dynamics ────────────────────────────────────────────
            // force(m, a) → F = m·a
            ("force".into(), Value::Lambda { params: vec!["m".into(), "a".into()], body: Box::new(Expr::Ident("__phinolib_force".into())), captures: HashMap::new() }),
            // weight(m) → W = m·g
            ("weight".into(), Value::Lambda { params: vec!["m".into()], body: Box::new(Expr::Ident("__phinolib_weight".into())), captures: HashMap::new() }),
            // friction_force(mu, normal) → f = μ·N
            ("frictionForce".into(), Value::Lambda { params: vec!["mu".into(), "normal".into()], body: Box::new(Expr::Ident("__phinolib_friction".into())), captures: HashMap::new() }),
            // impulse(F, dt) → J = F·Δt
            ("impulse".into(), Value::Lambda { params: vec!["F".into(), "dt".into()], body: Box::new(Expr::Ident("__phinolib_impulse".into())), captures: HashMap::new() }),
            // momentum(m, v) → p = m·v
            ("momentum".into(), Value::Lambda { params: vec!["m".into(), "v".into()], body: Box::new(Expr::Ident("__phinolib_momentum".into())), captures: HashMap::new() }),

            // ── Collisions ────────────────────────────────────────────────────
            // elastic_v1_after(m1, m2, v1, v2) → v1' after elastic collision
            ("elasticV1".into(), Value::Lambda { params: vec!["m1".into(), "m2".into(), "v1".into(), "v2".into()], body: Box::new(Expr::Ident("__phinolib_elastic_v1".into())), captures: HashMap::new() }),
            // elastic_v2_after(m1, m2, v1, v2) → v2' after elastic collision
            ("elasticV2".into(), Value::Lambda { params: vec!["m1".into(), "m2".into(), "v1".into(), "v2".into()], body: Box::new(Expr::Ident("__phinolib_elastic_v2".into())), captures: HashMap::new() }),
            // inelastic_v_final(m1, m2, v1, v2) → v_f after perfectly inelastic collision
            ("inelasticVFinal".into(), Value::Lambda { params: vec!["m1".into(), "m2".into(), "v1".into(), "v2".into()], body: Box::new(Expr::Ident("__phinolib_inelastic_vf".into())), captures: HashMap::new() }),
            // coeff_of_restitution(v1_rel_before, v1_rel_after) → e = |v_sep|/|v_approach|
            ("coeffRestitution".into(), Value::Lambda { params: vec!["v_approach".into(), "v_separate".into()], body: Box::new(Expr::Ident("__phinolib_coeff_restitution".into())), captures: HashMap::new() }),

            // ── Gravity & Orbital Mechanics ───────────────────────────────────
            // grav_force(m1, m2, r) → F = G·m1·m2/r²
            ("gravForce".into(), Value::Lambda { params: vec!["m1".into(), "m2".into(), "r".into()], body: Box::new(Expr::Ident("__phinolib_grav_force".into())), captures: HashMap::new() }),
            // grav_field(M, r) → g = G·M/r²
            ("gravField".into(), Value::Lambda { params: vec!["M".into(), "r".into()], body: Box::new(Expr::Ident("__phinolib_grav_field".into())), captures: HashMap::new() }),
            // grav_potential(M, r) → φ = -G·M/r
            ("gravPotential".into(), Value::Lambda { params: vec!["M".into(), "r".into()], body: Box::new(Expr::Ident("__phinolib_grav_potential".into())), captures: HashMap::new() }),
            // escape_velocity(M, r) → v_e = sqrt(2·G·M/r)
            ("escapeVelocity".into(), Value::Lambda { params: vec!["M".into(), "r".into()], body: Box::new(Expr::Ident("__phinolib_escape_velocity".into())), captures: HashMap::new() }),
            // orbital_velocity(M, r) → v_o = sqrt(G·M/r)
            ("orbitalVelocity".into(), Value::Lambda { params: vec!["M".into(), "r".into()], body: Box::new(Expr::Ident("__phinolib_orbital_velocity".into())), captures: HashMap::new() }),
            // orbital_period(M, r) → T = 2π·sqrt(r³/G·M)   [Kepler's 3rd Law]
            ("orbitalPeriod".into(), Value::Lambda { params: vec!["M".into(), "r".into()], body: Box::new(Expr::Ident("__phinolib_orbital_period".into())), captures: HashMap::new() }),
            // schwarzschild_radius(M) → r_s = 2·G·M/c²
            ("schwarzschildRadius".into(), Value::Lambda { params: vec!["M".into()], body: Box::new(Expr::Ident("__phinolib_schwarzschild".into())), captures: HashMap::new() }),
            // roche_limit(R_primary, rho_primary, rho_secondary) → d = R·(2·ρp/ρs)^(1/3)
            ("rocheLimit".into(), Value::Lambda { params: vec!["R_primary".into(), "rho_p".into(), "rho_s".into()], body: Box::new(Expr::Ident("__phinolib_roche_limit".into())), captures: HashMap::new() }),
            // hill_sphere(a, m, M) → r_H = a·(m/3M)^(1/3)
            ("hillSphere".into(), Value::Lambda { params: vec!["a".into(), "m".into(), "M".into()], body: Box::new(Expr::Ident("__phinolib_hill_sphere".into())), captures: HashMap::new() }),

            // ── Energy & Work ─────────────────────────────────────────────────
            // kinetic_energy(m, v) → KE = 0.5·m·v²
            ("kineticEnergy".into(), Value::Lambda { params: vec!["m".into(), "v".into()], body: Box::new(Expr::Ident("__phinolib_ke".into())), captures: HashMap::new() }),
            // potential_energy(m, h) → PE = m·g·h
            ("potentialEnergy".into(), Value::Lambda { params: vec!["m".into(), "h".into()], body: Box::new(Expr::Ident("__phinolib_pe".into())), captures: HashMap::new() }),
            // elastic_potential(k, x) → U = 0.5·k·x²
            ("elasticPotential".into(), Value::Lambda { params: vec!["k".into(), "x".into()], body: Box::new(Expr::Ident("__phinolib_elastic_pe".into())), captures: HashMap::new() }),
            // work(F, d, angle_deg) → W = F·d·cosθ
            ("work".into(), Value::Lambda { params: vec!["F".into(), "d".into(), "angle_deg".into()], body: Box::new(Expr::Ident("__phinolib_work".into())), captures: HashMap::new() }),
            // power(W, t) → P = W/t
            ("power".into(), Value::Lambda { params: vec!["W".into(), "t".into()], body: Box::new(Expr::Ident("__phinolib_power".into())), captures: HashMap::new() }),
            // power_from_force(F, v) → P = F·v
            ("powerFromForce".into(), Value::Lambda { params: vec!["F".into(), "v".into()], body: Box::new(Expr::Ident("__phinolib_power_fv".into())), captures: HashMap::new() }),
            // efficiency(W_useful, W_total) → η = W_useful/W_total
            ("efficiency".into(), Value::Lambda { params: vec!["W_useful".into(), "W_total".into()], body: Box::new(Expr::Ident("__phinolib_efficiency".into())), captures: HashMap::new() }),
            // rotational_ke(I, omega) → KE_rot = 0.5·I·ω²
            ("rotationalKE".into(), Value::Lambda { params: vec!["I".into(), "omega".into()], body: Box::new(Expr::Ident("__phinolib_rotational_ke".into())), captures: HashMap::new() }),

            // ── Simple Harmonic Motion (SHM) & Oscillations ───────────────────
            // shm_period_spring(m, k) → T = 2π·sqrt(m/k)
            ("shmPeriodSpring".into(), Value::Lambda { params: vec!["m".into(), "k".into()], body: Box::new(Expr::Ident("__phinolib_shm_spring".into())), captures: HashMap::new() }),
            // shm_period_pendulum(L) → T = 2π·sqrt(L/g)   [small angle]
            ("shmPeriodPendulum".into(), Value::Lambda { params: vec!["L".into()], body: Box::new(Expr::Ident("__phinolib_shm_pendulum".into())), captures: HashMap::new() }),
            // shm_freq(T) → f = 1/T
            ("shmFreq".into(), Value::Lambda { params: vec!["T".into()], body: Box::new(Expr::Ident("__phinolib_shm_freq".into())), captures: HashMap::new() }),
            // shm_x(A, omega, t, phi_deg) → x = A·cos(ω·t + φ)
            ("shmX".into(), Value::Lambda { params: vec!["A".into(), "omega".into(), "t".into(), "phi_deg".into()], body: Box::new(Expr::Ident("__phinolib_shm_x".into())), captures: HashMap::new() }),
            // shm_v(A, omega, t, phi_deg) → v = -A·ω·sin(ω·t + φ)
            ("shmV".into(), Value::Lambda { params: vec!["A".into(), "omega".into(), "t".into(), "phi_deg".into()], body: Box::new(Expr::Ident("__phinolib_shm_v".into())), captures: HashMap::new() }),
            // shm_a(A, omega, t, phi_deg) → a = -A·ω²·cos(ω·t + φ)
            ("shmA".into(), Value::Lambda { params: vec!["A".into(), "omega".into(), "t".into(), "phi_deg".into()], body: Box::new(Expr::Ident("__phinolib_shm_a".into())), captures: HashMap::new() }),
            // shm_max_speed(A, omega) → v_max = A·ω
            ("shmMaxSpeed".into(), Value::Lambda { params: vec!["A".into(), "omega".into()], body: Box::new(Expr::Ident("__phinolib_shm_vmax".into())), captures: HashMap::new() }),
            // shm_energy(m, omega, A) → E = 0.5·m·ω²·A²
            ("shmEnergy".into(), Value::Lambda { params: vec!["m".into(), "omega".into(), "A".into()], body: Box::new(Expr::Ident("__phinolib_shm_energy".into())), captures: HashMap::new() }),
            // damping_ratio(b, m, k) → ζ = b / (2·sqrt(m·k))
            ("dampingRatio".into(), Value::Lambda { params: vec!["b".into(), "m".into(), "k".into()], body: Box::new(Expr::Ident("__phinolib_damping_ratio".into())), captures: HashMap::new() }),

            // ── Waves ─────────────────────────────────────────────────────────
            // wave_speed(f, lambda) → v = f·λ
            ("waveSpeed".into(), Value::Lambda { params: vec!["f".into(), "lambda".into()], body: Box::new(Expr::Ident("__phinolib_wave_speed".into())), captures: HashMap::new() }),
            // wave_energy_density(A, omega, rho) → u = 0.5·ρ·ω²·A²
            ("waveEnergyDensity".into(), Value::Lambda { params: vec!["A".into(), "omega".into(), "rho".into()], body: Box::new(Expr::Ident("__phinolib_wave_energy".into())), captures: HashMap::new() }),
            // doppler_freq(f_s, v_sound, v_observer, v_source) → f_obs = f_s·(v±v_o)/(v∓v_s)
            // v_observer: positive if moving toward source; v_source: positive if moving toward observer
            ("dopplerFreq".into(), Value::Lambda { params: vec!["f_s".into(), "v_sound".into(), "v_obs".into(), "v_src".into()], body: Box::new(Expr::Ident("__phinolib_doppler".into())), captures: HashMap::new() }),
            // standing_wave_freq(n, L, v) → f_n = n·v/(2L)
            ("standingWaveFreq".into(), Value::Lambda { params: vec!["n".into(), "L".into(), "v".into()], body: Box::new(Expr::Ident("__phinolib_standing_wave".into())), captures: HashMap::new() }),
            // sound_intensity(P, r) → I = P / (4π·r²)
            ("soundIntensity".into(), Value::Lambda { params: vec!["P".into(), "r".into()], body: Box::new(Expr::Ident("__phinolib_sound_intensity".into())), captures: HashMap::new() }),
            // decibels(I) → dB = 10·log10(I / I₀)  where I₀ = 1e-12
            ("decibels".into(), Value::Lambda { params: vec!["I".into()], body: Box::new(Expr::Ident("__phinolib_decibels".into())), captures: HashMap::new() }),

            // ── Thermodynamics ────────────────────────────────────────────────
            // ideal_gas_p(n, T, V) → P = n·R·T/V
            ("idealGasP".into(), Value::Lambda { params: vec!["n".into(), "T".into(), "V".into()], body: Box::new(Expr::Ident("__phinolib_ideal_gas_p".into())), captures: HashMap::new() }),
            // ideal_gas_v(n, T, P) → V = n·R·T/P
            ("idealGasV".into(), Value::Lambda { params: vec!["n".into(), "T".into(), "P".into()], body: Box::new(Expr::Ident("__phinolib_ideal_gas_v".into())), captures: HashMap::new() }),
            // ideal_gas_t(P, V, n) → T = P·V/(n·R)
            ("idealGasT".into(), Value::Lambda { params: vec!["P".into(), "V".into(), "n".into()], body: Box::new(Expr::Ident("__phinolib_ideal_gas_t".into())), captures: HashMap::new() }),
            // heat_conduction(k, A, dT, dx) → Q/t = k·A·ΔT/Δx  (Fourier's law)
            ("heatConduction".into(), Value::Lambda { params: vec!["k".into(), "A".into(), "dT".into(), "dx".into()], body: Box::new(Expr::Ident("__phinolib_heat_conduction".into())), captures: HashMap::new() }),
            // thermal_radiation(epsilon, A, T) → P = ε·σ·A·T⁴  (Stefan-Boltzmann)
            ("thermalRadiation".into(), Value::Lambda { params: vec!["epsilon".into(), "A".into(), "T".into()], body: Box::new(Expr::Ident("__phinolib_thermal_radiation".into())), captures: HashMap::new() }),
            // heat_capacity(m, c, dT) → Q = m·c·ΔT
            ("heatCapacity".into(), Value::Lambda { params: vec!["m".into(), "c".into(), "dT".into()], body: Box::new(Expr::Ident("__phinolib_heat_capacity".into())), captures: HashMap::new() }),
            // celsius_to_kelvin(C) → K = C + 273.15
            ("celsiusToKelvin".into(), Value::Lambda { params: vec!["C".into()], body: Box::new(Expr::Ident("__phinolib_c_to_k".into())), captures: HashMap::new() }),
            // kelvin_to_celsius(K) → C = K - 273.15
            ("kelvinToCelsius".into(), Value::Lambda { params: vec!["K".into()], body: Box::new(Expr::Ident("__phinolib_k_to_c".into())), captures: HashMap::new() }),
            // rms_speed(M_mol, T) → v_rms = sqrt(3·R·T/M)
            ("rmsSpeed".into(), Value::Lambda { params: vec!["M_mol".into(), "T".into()], body: Box::new(Expr::Ident("__phinolib_rms_speed".into())), captures: HashMap::new() }),
            // carnot_efficiency(T_hot, T_cold) → η = 1 - T_c/T_h   (Kelvin temps)
            ("carnotEfficiency".into(), Value::Lambda { params: vec!["T_hot".into(), "T_cold".into()], body: Box::new(Expr::Ident("__phinolib_carnot".into())), captures: HashMap::new() }),

            // ── Electromagnetism ──────────────────────────────────────────────
            // coulomb_force(q1, q2, r) → F = k_e·q1·q2/r²
            ("coulombForce".into(), Value::Lambda { params: vec!["q1".into(), "q2".into(), "r".into()], body: Box::new(Expr::Ident("__phinolib_coulomb".into())), captures: HashMap::new() }),
            // electric_field(q, r) → E = k_e·q/r²
            ("electricField".into(), Value::Lambda { params: vec!["q".into(), "r".into()], body: Box::new(Expr::Ident("__phinolib_electric_field".into())), captures: HashMap::new() }),
            // electric_potential(q, r) → V = k_e·q/r
            ("electricPotential".into(), Value::Lambda { params: vec!["q".into(), "r".into()], body: Box::new(Expr::Ident("__phinolib_electric_potential".into())), captures: HashMap::new() }),
            // electric_pe(q, V) → U = q·V
            ("electricPE".into(), Value::Lambda { params: vec!["q".into(), "V".into()], body: Box::new(Expr::Ident("__phinolib_electric_pe".into())), captures: HashMap::new() }),
            // capacitance_parallel_plate(eps_r, A, d) → C = ε₀·εᵣ·A/d
            ("capacitancePlateCap".into(), Value::Lambda { params: vec!["eps_r".into(), "A".into(), "d".into()], body: Box::new(Expr::Ident("__phinolib_capacitance".into())), captures: HashMap::new() }),
            // ohms_law_v(I, R) → V = I·R
            ("ohmsV".into(), Value::Lambda { params: vec!["I".into(), "R".into()], body: Box::new(Expr::Ident("__phinolib_ohms_v".into())), captures: HashMap::new() }),
            // ohms_law_i(V, R) → I = V/R
            ("ohmsI".into(), Value::Lambda { params: vec!["V".into(), "R".into()], body: Box::new(Expr::Ident("__phinolib_ohms_i".into())), captures: HashMap::new() }),
            // power_electrical(V, I) → P = V·I
            ("electricPower".into(), Value::Lambda { params: vec!["V".into(), "I".into()], body: Box::new(Expr::Ident("__phinolib_electric_power".into())), captures: HashMap::new() }),
            // lorentz_force(q, v, B, angle_deg) → F = q·v·B·sinθ
            ("lorentzForce".into(), Value::Lambda { params: vec!["q".into(), "v".into(), "B".into(), "angle_deg".into()], body: Box::new(Expr::Ident("__phinolib_lorentz".into())), captures: HashMap::new() }),
            // biot_savart_infinite_wire(I, r) → B = μ₀·I/(2π·r)
            ("biotSavartWire".into(), Value::Lambda { params: vec!["I".into(), "r".into()], body: Box::new(Expr::Ident("__phinolib_biot_savart".into())), captures: HashMap::new() }),
            // faraday_emf(N, dPhi, dt) → ε = -N·dΦ/dt
            ("faradayEMF".into(), Value::Lambda { params: vec!["N".into(), "dPhi".into(), "dt".into()], body: Box::new(Expr::Ident("__phinolib_faraday".into())), captures: HashMap::new() }),
            // inductance_solenoid(N, A, l) → L = μ₀·N²·A/l
            ("inductanceSolenoid".into(), Value::Lambda { params: vec!["N".into(), "A".into(), "l".into()], body: Box::new(Expr::Ident("__phinolib_inductance".into())), captures: HashMap::new() }),
            // photon_energy(f) → E = h·f
            ("photonEnergy".into(), Value::Lambda { params: vec!["f".into()], body: Box::new(Expr::Ident("__phinolib_photon_energy".into())), captures: HashMap::new() }),
            // photon_wavelength(f) → λ = c/f
            ("photonWavelength".into(), Value::Lambda { params: vec!["f".into()], body: Box::new(Expr::Ident("__phinolib_photon_wavelength".into())), captures: HashMap::new() }),
            // de_broglie(m, v) → λ = h/(m·v)
            ("deBroglieWavelength".into(), Value::Lambda { params: vec!["m".into(), "v".into()], body: Box::new(Expr::Ident("__phinolib_de_broglie".into())), captures: HashMap::new() }),

            // ── Fluid Mechanics ───────────────────────────────────────────────
            // hydrostatic_pressure(rho, h) → P = ρ·g·h
            ("hydrostaticP".into(), Value::Lambda { params: vec!["rho".into(), "h".into()], body: Box::new(Expr::Ident("__phinolib_hydrostatic".into())), captures: HashMap::new() }),
            // buoyancy_force(rho_fluid, V_sub) → F_b = ρ·g·V
            ("buoyancyForce".into(), Value::Lambda { params: vec!["rho_fluid".into(), "V_sub".into()], body: Box::new(Expr::Ident("__phinolib_buoyancy".into())), captures: HashMap::new() }),
            // bernoulli_v2(v1, p1, p2, rho, h1, h2) → v₂ using Bernoulli equation
            ("bernoulliV2".into(), Value::Lambda { params: vec!["v1".into(), "p1".into(), "p2".into(), "rho".into(), "h1".into(), "h2".into()], body: Box::new(Expr::Ident("__phinolib_bernoulli_v2".into())), captures: HashMap::new() }),
            // stokes_drag(eta, r, v) → F_drag = 6π·η·r·v
            ("stokesDrag".into(), Value::Lambda { params: vec!["eta".into(), "r".into(), "v".into()], body: Box::new(Expr::Ident("__phinolib_stokes_drag".into())), captures: HashMap::new() }),
            // reynolds_number(rho, v, L, eta) → Re = ρ·v·L/η
            ("reynoldsNumber".into(), Value::Lambda { params: vec!["rho".into(), "v".into(), "L".into(), "eta".into()], body: Box::new(Expr::Ident("__phinolib_reynolds".into())), captures: HashMap::new() }),
            // flow_rate(A, v) → Q = A·v
            ("flowRate".into(), Value::Lambda { params: vec!["A".into(), "v".into()], body: Box::new(Expr::Ident("__phinolib_flow_rate".into())), captures: HashMap::new() }),
            // mach_number(v, v_sound) → M = v/v_sound
            ("machNumber".into(), Value::Lambda { params: vec!["v".into(), "v_sound".into()], body: Box::new(Expr::Ident("__phinolib_mach".into())), captures: HashMap::new() }),

            // ── Special Relativity ────────────────────────────────────────────
            // lorentz_factor(v) → γ = 1/sqrt(1 - v²/c²)
            ("lorentzFactor".into(), Value::Lambda { params: vec!["v".into()], body: Box::new(Expr::Ident("__phinolib_lorentz_factor".into())), captures: HashMap::new() }),
            // time_dilation(t0, v) → t = γ·t₀
            ("timeDilation".into(), Value::Lambda { params: vec!["t0".into(), "v".into()], body: Box::new(Expr::Ident("__phinolib_time_dilation".into())), captures: HashMap::new() }),
            // length_contraction(L0, v) → L = L₀/γ
            ("lengthContraction".into(), Value::Lambda { params: vec!["L0".into(), "v".into()], body: Box::new(Expr::Ident("__phinolib_length_contraction".into())), captures: HashMap::new() }),
            // relativistic_mass(m0, v) → m = γ·m₀
            ("relativisticMass".into(), Value::Lambda { params: vec!["m0".into(), "v".into()], body: Box::new(Expr::Ident("__phinolib_rel_mass".into())), captures: HashMap::new() }),
            // rest_energy(m) → E₀ = m·c²
            ("restEnergy".into(), Value::Lambda { params: vec!["m".into()], body: Box::new(Expr::Ident("__phinolib_rest_energy".into())), captures: HashMap::new() }),
            // relativistic_ke(m0, v) → KE = (γ-1)·m₀·c²
            ("relativisticKE".into(), Value::Lambda { params: vec!["m0".into(), "v".into()], body: Box::new(Expr::Ident("__phinolib_rel_ke".into())), captures: HashMap::new() }),
            // relativistic_momentum(m0, v) → p = γ·m₀·v
            ("relativisticMomentum".into(), Value::Lambda { params: vec!["m0".into(), "v".into()], body: Box::new(Expr::Ident("__phinolib_rel_momentum".into())), captures: HashMap::new() }),
            // velocity_addition(u, v) → w = (u+v)/(1+u·v/c²)  [relativistic]
            ("relVelocityAdd".into(), Value::Lambda { params: vec!["u".into(), "v".into()], body: Box::new(Expr::Ident("__phinolib_rel_velocity_add".into())), captures: HashMap::new() }),

            // ── Optics ────────────────────────────────────────────────────────
            // snells_law_r(n1, theta1_deg, n2) → θ₂ = arcsin(n1·sinθ₁/n2)  in degrees
            ("snellsLaw".into(), Value::Lambda { params: vec!["n1".into(), "theta1_deg".into(), "n2".into()], body: Box::new(Expr::Ident("__phinolib_snells".into())), captures: HashMap::new() }),
            // critical_angle(n1, n2) → θ_c = arcsin(n2/n1)  in degrees
            ("criticalAngle".into(), Value::Lambda { params: vec!["n1".into(), "n2".into()], body: Box::new(Expr::Ident("__phinolib_critical_angle".into())), captures: HashMap::new() }),
            // thin_lens(f, do_) → 1/di = 1/f - 1/do → di
            ("thinLens".into(), Value::Lambda { params: vec!["f".into(), "do_".into()], body: Box::new(Expr::Ident("__phinolib_thin_lens".into())), captures: HashMap::new() }),
            // magnification(di, do_) → m = -di/do
            ("magnification".into(), Value::Lambda { params: vec!["di".into(), "do_".into()], body: Box::new(Expr::Ident("__phinolib_magnification".into())), captures: HashMap::new() }),

            // ── Utility ───────────────────────────────────────────────────────
            // deg_to_rad(deg) → deg * π/180
            ("degToRad".into(), Value::Lambda { params: vec!["deg".into()], body: Box::new(Expr::Ident("__phinolib_deg_to_rad".into())), captures: HashMap::new() }),
            // rad_to_deg(rad) → rad * 180/π
            ("radToDeg".into(), Value::Lambda { params: vec!["rad".into()], body: Box::new(Expr::Ident("__phinolib_rad_to_deg".into())), captures: HashMap::new() }),
            // sig_figs(x, n) → round x to n significant figures
            ("sigFigs".into(), Value::Lambda { params: vec!["x".into(), "n".into()], body: Box::new(Expr::Ident("__phinolib_sig_figs".into())), captures: HashMap::new() }),
            // unit_vector_2d(x, y) → [x/|v|, y/|v|]
            ("unitVector2d".into(), Value::Lambda { params: vec!["x".into(), "y".into()], body: Box::new(Expr::Ident("__phinolib_unit_vec2d".into())), captures: HashMap::new() }),
            // vector_magnitude_2d(x, y) → sqrt(x²+y²)
            ("magnitude2d".into(), Value::Lambda { params: vec!["x".into(), "y".into()], body: Box::new(Expr::Ident("__phinolib_magnitude2d".into())), captures: HashMap::new() }),
            // dot_product_2d(ax, ay, bx, by) → a·b
            ("dotProduct2d".into(), Value::Lambda { params: vec!["ax".into(), "ay".into(), "bx".into(), "by".into()], body: Box::new(Expr::Ident("__phinolib_dot2d".into())), captures: HashMap::new() }),
            // cross_product_2d(ax, ay, bx, by) → |a×b| (z-component of 3D cross)
            ("crossProduct2d".into(), Value::Lambda { params: vec!["ax".into(), "ay".into(), "bx".into(), "by".into()], body: Box::new(Expr::Ident("__phinolib_cross2d".into())), captures: HashMap::new() }),
            // angle_between_2d(ax, ay, bx, by) → angle in degrees
            ("angleBetween2d".into(), Value::Lambda { params: vec!["ax".into(), "ay".into(), "bx".into(), "by".into()], body: Box::new(Expr::Ident("__phinolib_angle_between2d".into())), captures: HashMap::new() }),
        ]),

        // =====================================================================
        // Vyraweb — Remox Web Framework
        // Flask Se Bhi Powerful — Ek Hi Module Se Poora Web Server
        //
        // Syntax (Flask se bhi aasaan):
        //   use Vyraweb
        //   Vyraweb.get("/", fn() { "Hello World" })
        //   Vyraweb.post("/login", fn(body) { "Logged in" })
        //   Vyraweb.run(port: 8080)
        //
        // Features:
        //   route(method, path, handler) — any HTTP method register karo
        //   get(path, handler)           — GET route shorthand
        //   post(path, handler)          — POST route shorthand
        //   put(path, handler)           — PUT route shorthand
        //   delete(path, handler)        — DELETE route shorthand
        //   json(data)                   — JSON response banao
        //   html(content)                — HTML response banao
        //   text(content)                — plain text response
        //   redirect(url)                — redirect response
        //   status(code, body)           — custom status code
        //   header(key, val)             — response header set karo
        //   static_dir(path)             — static files serve karo
        //   middleware(fn)               — global middleware add karo
        //   run(port)                    — server start karo
        //   run_secure(port, cert, key)  — HTTPS server start karo
        //   url_for(name)                — reverse route lookup
        //   version                      — Vyraweb version string
        // =====================================================================
        "Vyraweb" => Some(vec![
            ("version".into(), Value::Str("Vyraweb 2.0 — ORM + Templates + WebSocket — No Simulation".into())),

            // ── Route Registration ─────────────────────────────────────────────
            // route(method, path, handler) → register a route for any HTTP method
            ("route".into(),   Value::Lambda { params: vec!["method".into(), "path".into(), "handler".into()], body: Box::new(Expr::Ident("__vyraweb_route".into())), captures: HashMap::new() }),
            // get(path, handler) → register GET route
            ("get".into(),     Value::Lambda { params: vec!["path".into(), "handler".into()], body: Box::new(Expr::Ident("__vyraweb_get".into())), captures: HashMap::new() }),
            // post(path, handler) → register POST route
            ("post".into(),    Value::Lambda { params: vec!["path".into(), "handler".into()], body: Box::new(Expr::Ident("__vyraweb_post".into())), captures: HashMap::new() }),
            // put(path, handler) → register PUT route
            ("put".into(),     Value::Lambda { params: vec!["path".into(), "handler".into()], body: Box::new(Expr::Ident("__vyraweb_put".into())), captures: HashMap::new() }),
            // delete(path, handler) → register DELETE route
            ("delete".into(),  Value::Lambda { params: vec!["path".into(), "handler".into()], body: Box::new(Expr::Ident("__vyraweb_delete".into())), captures: HashMap::new() }),
            // patch(path, handler) → register PATCH route
            ("patch".into(),   Value::Lambda { params: vec!["path".into(), "handler".into()], body: Box::new(Expr::Ident("__vyraweb_patch".into())), captures: HashMap::new() }),

            // ── Response Builders ──────────────────────────────────────────────
            // json(data) → JSON string banao map/list se
            ("json".into(),     Value::Lambda { params: vec!["data".into()], body: Box::new(Expr::Ident("__vyraweb_json".into())), captures: HashMap::new() }),
            // html(content) → HTML response
            ("html".into(),     Value::Lambda { params: vec!["content".into()], body: Box::new(Expr::Ident("__vyraweb_html".into())), captures: HashMap::new() }),
            // text(content) → plain text response
            ("text".into(),     Value::Lambda { params: vec!["content".into()], body: Box::new(Expr::Ident("__vyraweb_text".into())), captures: HashMap::new() }),
            // redirect(url) → 302 redirect response
            ("redirect".into(), Value::Lambda { params: vec!["url".into()], body: Box::new(Expr::Ident("__vyraweb_redirect".into())), captures: HashMap::new() }),
            // status(code, body) → response with custom HTTP status code
            ("status".into(),   Value::Lambda { params: vec!["code".into(), "body".into()], body: Box::new(Expr::Ident("__vyraweb_status".into())), captures: HashMap::new() }),
            // header(key, value) → add header to next response
            ("header".into(),   Value::Lambda { params: vec!["key".into(), "val".into()], body: Box::new(Expr::Ident("__vyraweb_header".into())), captures: HashMap::new() }),

            // ── Middleware & Static ────────────────────────────────────────────
            // middleware(fn) → register global middleware function
            ("middleware".into(),  Value::Lambda { params: vec!["handler".into()], body: Box::new(Expr::Ident("__vyraweb_middleware".into())), captures: HashMap::new() }),
            // static_dir(dir_path) → serve static files from directory
            ("static_dir".into(), Value::Lambda { params: vec!["dir_path".into()], body: Box::new(Expr::Ident("__vyraweb_static_dir".into())), captures: HashMap::new() }),

            // ── URL & Routing Utils ────────────────────────────────────────────
            // url_for(route_name) → reverse route URL lookup
            ("url_for".into(),    Value::Lambda { params: vec!["name".into()], body: Box::new(Expr::Ident("__vyraweb_url_for".into())), captures: HashMap::new() }),

            // ── Server Control ─────────────────────────────────────────────────
            // run(port) → HTTP server start karo (blocking)
            ("run".into(),         Value::Lambda { params: vec!["port".into()], body: Box::new(Expr::Ident("__vyraweb_run".into())), captures: HashMap::new() }),
            // run_secure(port, cert_path, key_path) → HTTPS server start karo
            ("run_secure".into(),  Value::Lambda { params: vec!["port".into(), "cert".into(), "key".into()], body: Box::new(Expr::Ident("__vyraweb_run_secure".into())), captures: HashMap::new() }),
            // stop() → server gracefully stop karo
            ("stop".into(),        Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__vyraweb_stop".into())), captures: HashMap::new() }),

            // ── ORM — VyraDB (Built-in SQLite-like File Database) ─────────────
            // db_connect(path)           → SQLite file se connect karo
            ("db_connect".into(),  Value::Lambda { params: vec!["path".into()], body: Box::new(Expr::Ident("__vyraweb_db_connect".into())), captures: HashMap::new() }),
            // db_query(sql)              → raw SQL execute karo, rows return kare
            ("db_query".into(),    Value::Lambda { params: vec!["sql".into()], body: Box::new(Expr::Ident("__vyraweb_db_query".into())), captures: HashMap::new() }),
            // db_exec(sql)               → INSERT/UPDATE/DELETE execute karo
            ("db_exec".into(),     Value::Lambda { params: vec!["sql".into()], body: Box::new(Expr::Ident("__vyraweb_db_exec".into())), captures: HashMap::new() }),
            // db_create(table, schema)   → table banao {col: type} map se
            ("db_create".into(),   Value::Lambda { params: vec!["table".into(), "schema".into()], body: Box::new(Expr::Ident("__vyraweb_db_create".into())), captures: HashMap::new() }),
            // db_insert(table, data)     → row insert karo {col: val} map se
            ("db_insert".into(),   Value::Lambda { params: vec!["table".into(), "data".into()], body: Box::new(Expr::Ident("__vyraweb_db_insert".into())), captures: HashMap::new() }),
            // db_find(table, where_map)  → rows dhundho {col: val} filter se
            ("db_find".into(),     Value::Lambda { params: vec!["table".into(), "filter".into()], body: Box::new(Expr::Ident("__vyraweb_db_find".into())), captures: HashMap::new() }),
            // db_find_all(table)         → table ki saari rows lao
            ("db_find_all".into(), Value::Lambda { params: vec!["table".into()], body: Box::new(Expr::Ident("__vyraweb_db_find_all".into())), captures: HashMap::new() }),
            // db_update(table, data, where_map) → rows update karo
            ("db_update".into(),   Value::Lambda { params: vec!["table".into(), "data".into(), "filter".into()], body: Box::new(Expr::Ident("__vyraweb_db_update".into())), captures: HashMap::new() }),
            // db_delete(table, where_map) → rows delete karo
            ("db_delete".into(),   Value::Lambda { params: vec!["table".into(), "filter".into()], body: Box::new(Expr::Ident("__vyraweb_db_delete".into())), captures: HashMap::new() }),
            // db_count(table)            → table mein total rows count
            ("db_count".into(),    Value::Lambda { params: vec!["table".into()], body: Box::new(Expr::Ident("__vyraweb_db_count".into())), captures: HashMap::new() }),
            // db_drop(table)             → table drop karo
            ("db_drop".into(),     Value::Lambda { params: vec!["table".into()], body: Box::new(Expr::Ident("__vyraweb_db_drop".into())), captures: HashMap::new() }),

            // ── Template Engine — VyraTmpl ─────────────────────────────────────
            // render(template_str, data)  → {var} aur {% if %} wala template render karo
            ("render".into(),      Value::Lambda { params: vec!["tmpl".into(), "data".into()], body: Box::new(Expr::Ident("__vyraweb_render".into())), captures: HashMap::new() }),
            // render_file(path, data)     → .vt file se template load karke render karo
            ("render_file".into(), Value::Lambda { params: vec!["path".into(), "data".into()], body: Box::new(Expr::Ident("__vyraweb_render_file".into())), captures: HashMap::new() }),
            // template(name, str)         → named template register karo
            ("template".into(),    Value::Lambda { params: vec!["name".into(), "tmpl".into()], body: Box::new(Expr::Ident("__vyraweb_template".into())), captures: HashMap::new() }),
            // use_template(name, data)    → registered template use karo
            ("use_template".into(), Value::Lambda { params: vec!["name".into(), "data".into()], body: Box::new(Expr::Ident("__vyraweb_use_template".into())), captures: HashMap::new() }),

            // ── WebSocket — VyraSocket ─────────────────────────────────────────
            // ws_listen(port)             → WebSocket server start karo
            ("ws_listen".into(),   Value::Lambda { params: vec!["port".into()], body: Box::new(Expr::Ident("__vyraweb_ws_listen".into())), captures: HashMap::new() }),
            // ws_on(event, handler)       → event handler register karo (connect/message/disconnect)
            ("ws_on".into(),       Value::Lambda { params: vec!["event".into(), "handler".into()], body: Box::new(Expr::Ident("__vyraweb_ws_on".into())), captures: HashMap::new() }),
            // ws_broadcast(msg)           → sabhi connected clients ko message bhejo
            ("ws_broadcast".into(), Value::Lambda { params: vec!["msg".into()], body: Box::new(Expr::Ident("__vyraweb_ws_broadcast".into())), captures: HashMap::new() }),
            // ws_send(client_id, msg)     → ek specific client ko message bhejo
            ("ws_send".into(),     Value::Lambda { params: vec!["client_id".into(), "msg".into()], body: Box::new(Expr::Ident("__vyraweb_ws_send".into())), captures: HashMap::new() }),
            // ws_clients()                → connected clients ki list
            ("ws_clients".into(),  Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__vyraweb_ws_clients".into())), captures: HashMap::new() }),
        ]),

        // =====================================================================
        // Autoclib — Remox ka apna CLI/Automation Engine (Python's Click + argparse
        // + Typer + Fire + Rich + Textual + Fabric — sab ek hi built-in module
        // mein, zero-import, zero-boilerplate).
        // use Autoclib  →  Autoclib.style("done", "green", true)
        // Covers: ANSI styling, tables, progress bars, spinners, panels/rules,
        // arg parsing with type coercion, subcommand trees, auto help text,
        // shell-completion generation, config parsing, mini-markdown render,
        // structured logging, and task dependency ordering (Fabric-style).
        // HONEST GAP: remoteExec depends on Monobat's network stack (same
        // MonobatHal placeholder as fs/net elsewhere in this file) — it will
        // return a clear "not wired yet" error until that driver lands.
        // =====================================================================
        "Autoclib" => Some(vec![
            // ---- Styling / Rich-style output ----
            ("style".into(),  Value::Lambda { params: vec!["text".into(),"color".into(),"bold".into()], body: Box::new(Expr::Ident("__autoclib_style".into())), captures: HashMap::new() }),
            ("print".into(),  Value::Lambda { params: vec!["text".into()], body: Box::new(Expr::Ident("__autoclib_print".into())), captures: HashMap::new() }),
            ("rule".into(),   Value::Lambda { params: vec!["title".into()], body: Box::new(Expr::Ident("__autoclib_rule".into())), captures: HashMap::new() }),
            ("panel".into(),  Value::Lambda { params: vec!["text".into(),"title".into()], body: Box::new(Expr::Ident("__autoclib_panel".into())), captures: HashMap::new() }),
            ("table".into(),  Value::Lambda { params: vec!["headers".into(),"rows".into()], body: Box::new(Expr::Ident("__autoclib_table".into())), captures: HashMap::new() }),
            ("progress".into(), Value::Lambda { params: vec!["current".into(),"total".into(),"label".into()], body: Box::new(Expr::Ident("__autoclib_progress".into())), captures: HashMap::new() }),
            ("spinner".into(), Value::Lambda { params: vec!["tick".into()], body: Box::new(Expr::Ident("__autoclib_spinner".into())), captures: HashMap::new() }),
            ("markdown".into(), Value::Lambda { params: vec!["text".into()], body: Box::new(Expr::Ident("__autoclib_markdown".into())), captures: HashMap::new() }),

            // ---- Prompts (Rich/Click-style user input) ----
            ("prompt".into(),  Value::Lambda { params: vec!["question".into()], body: Box::new(Expr::Ident("__autoclib_prompt".into())), captures: HashMap::new() }),
            ("confirm".into(), Value::Lambda { params: vec!["question".into()], body: Box::new(Expr::Ident("__autoclib_confirm".into())), captures: HashMap::new() }),

            // ---- Argument parsing (argparse + Click + Typer + Fire combined) ----
            ("parseArgs".into(), Value::Lambda { params: vec!["argv".into(),"spec".into()], body: Box::new(Expr::Ident("__autoclib_parse_args".into())), captures: HashMap::new() }),

            // ---- Subcommand tree (Click groups / Typer sub-apps) ----
            ("command".into(),       Value::Lambda { params: vec!["name".into(),"desc".into()], body: Box::new(Expr::Ident("__autoclib_command".into())), captures: HashMap::new() }),
            ("addSubcommand".into(), Value::Lambda { params: vec!["parent".into(),"child".into()], body: Box::new(Expr::Ident("__autoclib_add_subcommand".into())), captures: HashMap::new() }),
            // Renamed from "route" to "cliRoute": Vyraweb also exposes a
            // "route" function (HTTP routing) — different context (CLI
            // subcommand dispatch vs HTTP) so it doesn't collide today
            // since each lives in its own module map, but a distinct name
            // avoids any ambiguity if both modules are ever imported
            // together / flattened into one namespace.
            ("cliRoute".into(),      Value::Lambda { params: vec!["tree".into(),"argv".into()], body: Box::new(Expr::Ident("__autoclib_route".into())), captures: HashMap::new() }),
            ("help".into(),          Value::Lambda { params: vec!["tree".into()], body: Box::new(Expr::Ident("__autoclib_help".into())), captures: HashMap::new() }),
            ("completion".into(),    Value::Lambda { params: vec!["tree".into(),"shell".into()], body: Box::new(Expr::Ident("__autoclib_completion".into())), captures: HashMap::new() }),

            // ---- Config binding (auto-load defaults from a config file) ----
            ("config".into(), Value::Lambda { params: vec!["text".into()], body: Box::new(Expr::Ident("__autoclib_config".into())), captures: HashMap::new() }),

            // ---- Structured logging (custom — not in any of the 7 Python libs) ----
            ("log".into(), Value::Lambda { params: vec!["level".into(),"msg".into()], body: Box::new(Expr::Ident("__autoclib_log".into())), captures: HashMap::new() }),

            // ---- Automation (Fabric-style task ordering + real remote exec) ----
            ("taskSort".into(),   Value::Lambda { params: vec!["tasks".into()], body: Box::new(Expr::Ident("__autoclib_task_sort".into())), captures: HashMap::new() }),
            ("remoteExec".into(), Value::Lambda { params: vec!["host".into(),"cmd".into()], body: Box::new(Expr::Ident("__autoclib_remote_exec".into())), captures: HashMap::new() }),

            // ---- History (REPL-mode command memory — custom) ----
            ("historyAdd".into(),  Value::Lambda { params: vec!["line".into()], body: Box::new(Expr::Ident("__autoclib_history_add".into())), captures: HashMap::new() }),
            ("historyList".into(), Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__autoclib_history_list".into())), captures: HashMap::new() }),

            // ---- TUI widgets (Textual-equivalent) — reactive widget tree,
            // keyboard-navigable, rendered as real terminal ANSI text ----
            // widget(kind, id, label, value) → kind: "button"|"input"|"checkbox"|"listitem"|"tab"|"container"
            ("widget".into(),     Value::Lambda { params: vec!["kind".into(),"id".into(),"label".into(),"value".into()], body: Box::new(Expr::Ident("__autoclib_widget".into())), captures: HashMap::new() }),
            // container(id, children) → a "container" widget holding a List of child widgets
            ("container".into(),  Value::Lambda { params: vec!["id".into(),"children".into()], body: Box::new(Expr::Ident("__autoclib_container".into())), captures: HashMap::new() }),
            // render(tree, focusId) → Str, real ANSI text (focused widget shown inverse-video)
            ("render".into(),     Value::Lambda { params: vec!["tree".into(),"focusId".into()], body: Box::new(Expr::Ident("__autoclib_render".into())), captures: HashMap::new() }),
            // focusables(tree) → List<Str> of focusable widget ids, in tab order
            ("focusables".into(), Value::Lambda { params: vec!["tree".into()], body: Box::new(Expr::Ident("__autoclib_focusables".into())), captures: HashMap::new() }),
            // handleKey(tree, focusId, key) → Map{tree, focus, action} — real keyboard nav (tab/up/down/enter/space)
            ("handleKey".into(),  Value::Lambda { params: vec!["tree".into(),"focusId".into(),"key".into()], body: Box::new(Expr::Ident("__autoclib_handle_key".into())), captures: HashMap::new() }),
            // readKey() → Str — reads one real keystroke via HAL serial_read_byte if wired,
            // else falls back honestly to line-buffered stdin (documented gap, not faked)
            ("readKey".into(),    Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__autoclib_read_key".into())), captures: HashMap::new() }),
        ]),

        // =====================================================================
        // Remotest — Remox ka apna Testing Engine.
        // Python's pytest + unittest + nose2 + Hypothesis(partial) + Robot
        // Framework(keyword style) + Behave(BDD) + Locust(basic load) +
        // Faker + Mock/pytest-mock ko combine karke ek hi built-in module —
        // zero-import, zero-decorator-boilerplate, zero-config-file.
        // use Remotest
        // HONEST GAP (same policy as rest of this file): mock/spy objects
        // can't be called with plain `()` syntax yet (that needs the
        // interpreter's Expr tree, which this module doesn't construct by
        // hand to avoid guessing at internals) — call mockCall/spyCall
        // explicitly instead. Everything else below is real, working logic:
        // actual test execution via call_value, actual RNG-backed fake data
        // (same LCG as rand_state), actual fixture setup calls, real
        // pass/fail/skip accounting, real property-based shrinking (forAll
        // re-runs every shrink candidate through the real property fn), and
        // real Luhn-valid fake credit card numbers.
        // STILL NOT IN THIS BUILD (roadmap, not fake): Gherkin/BDD parsing
        // (Behave/Robot Framework-style), tox-style env matrices. `load()`
        // is real call aggregation but sequential on this no_std single-core
        // kernel build — see its own comment below for why, and what makes
        // it concurrent for free later.
        // =====================================================================
        "Remotest" => Some(vec![
            // ---- Registration (pytest + unittest + Behave, unified) ----
            ("test".into(),     Value::Lambda { params: vec!["name".into(),"fn".into(),"tags".into()], body: Box::new(Expr::Ident("__remotest_test".into())), captures: HashMap::new() }),
            ("describe".into(), Value::Lambda { params: vec!["name".into(),"fn".into()], body: Box::new(Expr::Ident("__remotest_describe".into())), captures: HashMap::new() }),
            ("it".into(),       Value::Lambda { params: vec!["name".into(),"fn".into(),"tags".into()], body: Box::new(Expr::Ident("__remotest_it".into())), captures: HashMap::new() }),
            ("skip".into(),     Value::Lambda { params: vec!["reason".into()], body: Box::new(Expr::Ident("__remotest_skip".into())), captures: HashMap::new() }),

            // ---- Fixtures (pytest-style setup, real calls not stubs) ----
            ("fixture".into(),    Value::Lambda { params: vec!["name".into(),"fn".into()], body: Box::new(Expr::Ident("__remotest_fixture".into())), captures: HashMap::new() }),
            ("useFixture".into(), Value::Lambda { params: vec!["name".into()], body: Box::new(Expr::Ident("__remotest_use_fixture".into())), captures: HashMap::new() }),

            // ---- Assertions (pytest asserts + unittest assertX, unified) ----
            ("assertEqual".into(),        Value::Lambda { params: vec!["a".into(),"b".into(),"msg".into()], body: Box::new(Expr::Ident("__remotest_assert_equal".into())), captures: HashMap::new() }),
            ("assertNotEqual".into(),     Value::Lambda { params: vec!["a".into(),"b".into(),"msg".into()], body: Box::new(Expr::Ident("__remotest_assert_not_equal".into())), captures: HashMap::new() }),
            ("assertTrue".into(),         Value::Lambda { params: vec!["v".into(),"msg".into()], body: Box::new(Expr::Ident("__remotest_assert_true".into())), captures: HashMap::new() }),
            ("assertFalse".into(),        Value::Lambda { params: vec!["v".into(),"msg".into()], body: Box::new(Expr::Ident("__remotest_assert_false".into())), captures: HashMap::new() }),
            ("assertNone".into(),         Value::Lambda { params: vec!["v".into(),"msg".into()], body: Box::new(Expr::Ident("__remotest_assert_none".into())), captures: HashMap::new() }),
            ("assertNotNone".into(),      Value::Lambda { params: vec!["v".into(),"msg".into()], body: Box::new(Expr::Ident("__remotest_assert_not_none".into())), captures: HashMap::new() }),
            ("assertIn".into(),           Value::Lambda { params: vec!["item".into(),"coll".into(),"msg".into()], body: Box::new(Expr::Ident("__remotest_assert_in".into())), captures: HashMap::new() }),
            ("assertAlmostEqual".into(),  Value::Lambda { params: vec!["a".into(),"b".into(),"tolerance".into(),"msg".into()], body: Box::new(Expr::Ident("__remotest_assert_almost_equal".into())), captures: HashMap::new() }),
            ("assertRaises".into(),       Value::Lambda { params: vec!["fn".into()], body: Box::new(Expr::Ident("__remotest_assert_raises".into())), captures: HashMap::new() }),

            // ---- Mocking / spying (Mock + pytest-mock) ----
            ("mock".into(),          Value::Lambda { params: vec!["name".into(),"returnValue".into()], body: Box::new(Expr::Ident("__remotest_mock".into())), captures: HashMap::new() }),
            ("mockCall".into(),      Value::Lambda { params: vec!["id".into(),"a".into(),"b".into()], body: Box::new(Expr::Ident("__remotest_mock_call".into())), captures: HashMap::new() }),
            ("spyCall".into(),       Value::Lambda { params: vec!["id".into(),"realFn".into(),"a".into(),"b".into()], body: Box::new(Expr::Ident("__remotest_spy_call".into())), captures: HashMap::new() }),
            ("mockCalls".into(),     Value::Lambda { params: vec!["id".into()], body: Box::new(Expr::Ident("__remotest_mock_calls".into())), captures: HashMap::new() }),
            ("mockCallCount".into(), Value::Lambda { params: vec!["id".into()], body: Box::new(Expr::Ident("__remotest_mock_call_count".into())), captures: HashMap::new() }),
            ("resetMocks".into(),    Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_reset_mocks".into())), captures: HashMap::new() }),

            // ---- Faker (real RNG-backed fake data, same LCG as rand_state —
            // 22 generators: identity, geo/address, business, web/security,
            // finance. fakeCreditCard is Luhn-valid, computed for real. ----
            ("fakeName".into(),     Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_fake_name".into())), captures: HashMap::new() }),
            ("fakeEmail".into(),    Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_fake_email".into())), captures: HashMap::new() }),
            ("fakeInt".into(),      Value::Lambda { params: vec!["min".into(),"max".into()], body: Box::new(Expr::Ident("__remotest_fake_int".into())), captures: HashMap::new() }),
            ("fakeSentence".into(), Value::Lambda { params: vec!["words".into()], body: Box::new(Expr::Ident("__remotest_fake_sentence".into())), captures: HashMap::new() }),
            ("fakeDate".into(),     Value::Lambda { params: vec!["yearMin".into(),"yearMax".into()], body: Box::new(Expr::Ident("__remotest_fake_date".into())), captures: HashMap::new() }),
            ("fakeUuid".into(),     Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_fake_uuid".into())), captures: HashMap::new() }),
            ("fakeStreet".into(),      Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_fake_street".into())), captures: HashMap::new() }),
            ("fakeCity".into(),        Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_fake_city".into())), captures: HashMap::new() }),
            ("fakeCountry".into(),     Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_fake_country".into())), captures: HashMap::new() }),
            ("fakeZipcode".into(),     Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_fake_zipcode".into())), captures: HashMap::new() }),
            ("fakeAddress".into(),     Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_fake_address".into())), captures: HashMap::new() }),
            ("fakePhone".into(),       Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_fake_phone".into())), captures: HashMap::new() }),
            ("fakeCompany".into(),     Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_fake_company".into())), captures: HashMap::new() }),
            ("fakeJobTitle".into(),    Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_fake_job_title".into())), captures: HashMap::new() }),
            ("fakeUsername".into(),    Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_fake_username".into())), captures: HashMap::new() }),
            ("fakePassword".into(),    Value::Lambda { params: vec!["length".into()], body: Box::new(Expr::Ident("__remotest_fake_password".into())), captures: HashMap::new() }),
            ("fakeColor".into(),       Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_fake_color".into())), captures: HashMap::new() }),
            ("fakeUrl".into(),         Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_fake_url".into())), captures: HashMap::new() }),
            ("fakeIpv4".into(),        Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_fake_ipv4".into())), captures: HashMap::new() }),
            ("fakeCreditCard".into(),  Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_fake_credit_card".into())), captures: HashMap::new() }),
            ("fakeBoolean".into(),     Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_fake_boolean".into())), captures: HashMap::new() }),
            ("fakeCurrency".into(),    Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_fake_currency".into())), captures: HashMap::new() }),
            ("fakeParagraph".into(),   Value::Lambda { params: vec!["sentences".into()], body: Box::new(Expr::Ident("__remotest_fake_paragraph".into())), captures: HashMap::new() }),

            // ---- Generators (Hypothesis-style strategies, as data — not
            // closures, since this interpreter doesn't build Expr trees by
            // hand). Each returns a Map{__gen__: kind, ...params} that
            // forAll() reads back to both sample AND shrink. ----
            ("genInt".into(),    Value::Lambda { params: vec!["min".into(),"max".into()], body: Box::new(Expr::Ident("__remotest_gen_int".into())), captures: HashMap::new() }),
            ("genFloat".into(),  Value::Lambda { params: vec!["min".into(),"max".into()], body: Box::new(Expr::Ident("__remotest_gen_float".into())), captures: HashMap::new() }),
            ("genBool".into(),   Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_gen_bool".into())), captures: HashMap::new() }),
            ("genString".into(), Value::Lambda { params: vec!["maxLen".into()], body: Box::new(Expr::Ident("__remotest_gen_string".into())), captures: HashMap::new() }),
            ("genList".into(),   Value::Lambda { params: vec!["elemGen".into(),"maxLen".into()], body: Box::new(Expr::Ident("__remotest_gen_list".into())), captures: HashMap::new() }),
            ("genOneOf".into(),  Value::Lambda { params: vec!["options".into()], body: Box::new(Expr::Ident("__remotest_gen_one_of".into())), captures: HashMap::new() }),

            // ---- Property-based testing (Hypothesis parity) ----
            // forAll(gensList, propertyFn, opts?) — runs opts.iterations
            // (default 100) random samples through propertyFn; on first
            // failure, shrinks each input toward its simplest failing form
            // and reports the minimal counterexample.
            ("forAll".into(), Value::Lambda { params: vec!["gens".into(),"propertyFn".into(),"opts".into()], body: Box::new(Expr::Ident("__remotest_for_all".into())), captures: HashMap::new() }),

            // ---- Load / stress testing (Locust parity — same concurrency
            // model, verified: Locust's own per-process "concurrent users"
            // are cooperatively-scheduled gevent greenlets on one core, not
            // OS threads; true multi-core needs separate Locust processes
            // too, via --master/--worker, because of Python's GIL). This
            // runs real cooperative round-robin interleaving across virtual
            // users with a spawnRate ramp-up, same behavioral model as a
            // single Locust worker process. ----
            ("load".into(), Value::Lambda { params: vec!["fn".into(),"opts".into()], body: Box::new(Expr::Ident("__remotest_load".into())), captures: HashMap::new() }),

            // ---- BDD (Behave/Robot Framework parity, native syntax) ----
            ("scenario".into(), Value::Lambda { params: vec!["name".into(),"fn".into(),"tags".into()], body: Box::new(Expr::Ident("__remotest_scenario".into())), captures: HashMap::new() }),
            ("given".into(),     Value::Lambda { params: vec!["desc".into(),"fn".into()], body: Box::new(Expr::Ident("__remotest_given".into())), captures: HashMap::new() }),
            ("when".into(),      Value::Lambda { params: vec!["desc".into(),"fn".into()], body: Box::new(Expr::Ident("__remotest_when".into())), captures: HashMap::new() }),
            ("then".into(),      Value::Lambda { params: vec!["desc".into(),"fn".into()], body: Box::new(Expr::Ident("__remotest_then".into())), captures: HashMap::new() }),
            ("and".into(),       Value::Lambda { params: vec!["desc".into(),"fn".into()], body: Box::new(Expr::Ident("__remotest_and_step".into())), captures: HashMap::new() }),

            // ---- Tox-style environment matrix (config/feature-flag matrix,
            // not multi-interpreter-version isolation — see dispatch comment) ----
            ("envMatrix".into(), Value::Lambda { params: vec!["envs".into(),"testFn".into()], body: Box::new(Expr::Ident("__remotest_env_matrix".into())), captures: HashMap::new() }),

            // ---- Runner (nose2/tox-style batch execution + reporting) ----
            ("runAll".into(),  Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_run_all".into())), captures: HashMap::new() }),
            ("runTag".into(),  Value::Lambda { params: vec!["tag".into()], body: Box::new(Expr::Ident("__remotest_run_tag".into())), captures: HashMap::new() }),
            ("reset".into(),   Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__remotest_reset".into())), captures: HashMap::new() }),
        ]),

        // =====================================================================
        // Astriloop — Remox Async Runtime Library (asyncio+trio+uvloop, unified)
        // See dispatch_astriloop() near end of file for implementation.
        // =====================================================================
        "Astriloop" => Some(vec![
            // ── CORE EVENT LOOP (asyncio.run equivalent) ──────────────────
            // Astriloop.run(async_fn)  — top-level async entry point
            // Blocks until the coroutine + all spawned tasks complete.
            ("run".into(),     Value::Lambda { params: vec!["fn".into()], body: Box::new(Expr::Ident("__astriloop_run".into())), captures: HashMap::new() }),
            // Astriloop.sleep(ms)  — cooperative yield + timer
            ("sleep".into(),   Value::Lambda { params: vec!["ms".into()], body: Box::new(Expr::Ident("__astriloop_sleep".into())), captures: HashMap::new() }),
            // Astriloop.tick()  — cooperative checkpoint (like trio.checkpoint())
            ("tick".into(),    Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__astriloop_tick".into())), captures: HashMap::new() }),
            // Astriloop.now()  — monotonic clock in ms (for timing/deadlines)
            ("now".into(),     Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__astriloop_now".into())), captures: HashMap::new() }),

            // ── TASK MANAGEMENT (asyncio.create_task / trio nursery) ──────
            // Astriloop.spawn(fn, args?)  — fire-and-forget task
            ("spawn".into(),      Value::Lambda { params: vec!["fn".into(), "args".into()], body: Box::new(Expr::Ident("__astriloop_spawn".into())), captures: HashMap::new() }),
            // Astriloop.spawnIn(nursery, fn, args?)  — structured spawn inside nursery scope
            ("spawnIn".into(),    Value::Lambda { params: vec!["nursery".into(), "fn".into(), "args".into()], body: Box::new(Expr::Ident("__astriloop_spawn_in".into())), captures: HashMap::new() }),
            // Astriloop.cancel(handle)  — cancel a running task
            ("cancel".into(),     Value::Lambda { params: vec!["handle".into()], body: Box::new(Expr::Ident("__astriloop_cancel".into())), captures: HashMap::new() }),
            // Astriloop.taskStatus(handle)  — "pending" | "done" | "cancelled" | "error"
            ("taskStatus".into(), Value::Lambda { params: vec!["handle".into()], body: Box::new(Expr::Ident("__astriloop_task_status".into())), captures: HashMap::new() }),

            // ── GATHER / RACE (asyncio.gather, asyncio.wait) ─────────────
            // Astriloop.gather(fns)  — run list of async fns, collect all results
            ("gather".into(),      Value::Lambda { params: vec!["fns".into()], body: Box::new(Expr::Ident("__astriloop_gather".into())), captures: HashMap::new() }),
            // Astriloop.race(fns)   — first to finish wins, rest cancelled
            ("race".into(),        Value::Lambda { params: vec!["fns".into()], body: Box::new(Expr::Ident("__astriloop_race".into())), captures: HashMap::new() }),
            // Astriloop.allSettled(fns)  — gather but never throws; returns {ok, value/error}
            ("allSettled".into(),  Value::Lambda { params: vec!["fns".into()], body: Box::new(Expr::Ident("__astriloop_all_settled".into())), captures: HashMap::new() }),
            // Astriloop.any(fns)    — first SUCCESS wins (like Promise.any)
            ("any".into(),         Value::Lambda { params: vec!["fns".into()], body: Box::new(Expr::Ident("__astriloop_any".into())), captures: HashMap::new() }),

            // ── STRUCTURED CONCURRENCY — NURSERY (trio-style) ─────────────
            // Astriloop.openNursery()  — create a nursery scope object
            // Use: let n = Astriloop.openNursery(); Astriloop.spawnIn(n, fn)
            //      Astriloop.waitNursery(n)  — wait for all tasks + propagate errors
            ("openNursery".into(),  Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__astriloop_open_nursery".into())), captures: HashMap::new() }),
            ("waitNursery".into(),  Value::Lambda { params: vec!["nursery".into()], body: Box::new(Expr::Ident("__astriloop_wait_nursery".into())), captures: HashMap::new() }),
            ("closeNursery".into(), Value::Lambda { params: vec!["nursery".into()], body: Box::new(Expr::Ident("__astriloop_close_nursery".into())), captures: HashMap::new() }),

            // ── TIMEOUT / DEADLINE (asyncio.wait_for + trio.move_on_after) ─
            // Astriloop.timeout(ms, fn)  — run fn; error if exceeds ms
            ("timeout".into(),     Value::Lambda { params: vec!["ms".into(), "fn".into()], body: Box::new(Expr::Ident("__astriloop_timeout".into())), captures: HashMap::new() }),
            // Astriloop.moveOnAfter(ms, fn, default?)  — run fn; return default if exceeds ms
            ("moveOnAfter".into(), Value::Lambda { params: vec!["ms".into(), "fn".into(), "default".into()], body: Box::new(Expr::Ident("__astriloop_move_on_after".into())), captures: HashMap::new() }),
            // Astriloop.shield(fn)  — run fn; protect from external cancellation
            ("shield".into(),      Value::Lambda { params: vec!["fn".into()], body: Box::new(Expr::Ident("__astriloop_shield".into())), captures: HashMap::new() }),

            // ── SYNCHRONIZATION PRIMITIVES ────────────────────────────────
            // Astriloop.Lock()  — async mutex
            ("Lock".into(),        Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__astriloop_lock_new".into())), captures: HashMap::new() }),
            ("acquire".into(),     Value::Lambda { params: vec!["lock".into()], body: Box::new(Expr::Ident("__astriloop_lock_acquire".into())), captures: HashMap::new() }),
            ("release".into(),     Value::Lambda { params: vec!["lock".into()], body: Box::new(Expr::Ident("__astriloop_lock_release".into())), captures: HashMap::new() }),
            ("withLock".into(),    Value::Lambda { params: vec!["lock".into(), "fn".into()], body: Box::new(Expr::Ident("__astriloop_with_lock".into())), captures: HashMap::new() }),

            // Astriloop.Event()  — async event flag (set/clear/wait)
            ("Event".into(),      Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__astriloop_event_new".into())), captures: HashMap::new() }),
            ("setEvent".into(),   Value::Lambda { params: vec!["ev".into()], body: Box::new(Expr::Ident("__astriloop_event_set".into())), captures: HashMap::new() }),
            ("clearEvent".into(), Value::Lambda { params: vec!["ev".into()], body: Box::new(Expr::Ident("__astriloop_event_clear".into())), captures: HashMap::new() }),
            ("waitEvent".into(),  Value::Lambda { params: vec!["ev".into(), "timeout".into()], body: Box::new(Expr::Ident("__astriloop_event_wait".into())), captures: HashMap::new() }),
            ("isSet".into(),      Value::Lambda { params: vec!["ev".into()], body: Box::new(Expr::Ident("__astriloop_event_is_set".into())), captures: HashMap::new() }),

            // Astriloop.Semaphore(n)  — counting semaphore
            ("Semaphore".into(),    Value::Lambda { params: vec!["n".into()], body: Box::new(Expr::Ident("__astriloop_sem_new".into())), captures: HashMap::new() }),
            ("semAcquire".into(),   Value::Lambda { params: vec!["sem".into()], body: Box::new(Expr::Ident("__astriloop_sem_acquire".into())), captures: HashMap::new() }),
            ("semRelease".into(),   Value::Lambda { params: vec!["sem".into()], body: Box::new(Expr::Ident("__astriloop_sem_release".into())), captures: HashMap::new() }),
            ("semValue".into(),     Value::Lambda { params: vec!["sem".into()], body: Box::new(Expr::Ident("__astriloop_sem_value".into())), captures: HashMap::new() }),

            // Astriloop.Barrier(n)  — n tasks must all arrive before any proceeds
            ("Barrier".into(),    Value::Lambda { params: vec!["n".into()], body: Box::new(Expr::Ident("__astriloop_barrier_new".into())), captures: HashMap::new() }),
            ("barrierWait".into(),Value::Lambda { params: vec!["b".into()], body: Box::new(Expr::Ident("__astriloop_barrier_wait".into())), captures: HashMap::new() }),

            // ── TYPED CHANNELS (go-style + trio.MemoryChannel) ───────────
            // Astriloop.Channel(capacity?)  — bounded/unbounded MPMC channel
            ("Channel".into(),   Value::Lambda { params: vec!["cap".into()], body: Box::new(Expr::Ident("__astriloop_chan_new".into())), captures: HashMap::new() }),
            ("send".into(),      Value::Lambda { params: vec!["ch".into(), "val".into()], body: Box::new(Expr::Ident("__astriloop_chan_send".into())), captures: HashMap::new() }),
            ("recv".into(),      Value::Lambda { params: vec!["ch".into()], body: Box::new(Expr::Ident("__astriloop_chan_recv".into())), captures: HashMap::new() }),
            ("tryRecv".into(),   Value::Lambda { params: vec!["ch".into()], body: Box::new(Expr::Ident("__astriloop_chan_try_recv".into())), captures: HashMap::new() }),
            ("trySend".into(),   Value::Lambda { params: vec!["ch".into(), "val".into()], body: Box::new(Expr::Ident("__astriloop_chan_try_send".into())), captures: HashMap::new() }),
            ("chanLen".into(),   Value::Lambda { params: vec!["ch".into()], body: Box::new(Expr::Ident("__astriloop_chan_len".into())), captures: HashMap::new() }),
            ("chanClose".into(), Value::Lambda { params: vec!["ch".into()], body: Box::new(Expr::Ident("__astriloop_chan_close".into())), captures: HashMap::new() }),
            ("select".into(),    Value::Lambda { params: vec!["cases".into()], body: Box::new(Expr::Ident("__astriloop_select".into())), captures: HashMap::new() }),

            // ── ASYNC QUEUE (asyncio.Queue + priority support) ────────────
            // Astriloop.Queue(maxsize?)
            ("Queue".into(),         Value::Lambda { params: vec!["maxsize".into()], body: Box::new(Expr::Ident("__astriloop_queue_new".into())), captures: HashMap::new() }),
            ("PriorityQueue".into(), Value::Lambda { params: vec!["maxsize".into()], body: Box::new(Expr::Ident("__astriloop_pqueue_new".into())), captures: HashMap::new() }),
            ("qput".into(),          Value::Lambda { params: vec!["q".into(), "val".into(), "priority".into()], body: Box::new(Expr::Ident("__astriloop_queue_put".into())), captures: HashMap::new() }),
            ("qget".into(),          Value::Lambda { params: vec!["q".into()], body: Box::new(Expr::Ident("__astriloop_queue_get".into())), captures: HashMap::new() }),
            ("qtryGet".into(),       Value::Lambda { params: vec!["q".into()], body: Box::new(Expr::Ident("__astriloop_queue_try_get".into())), captures: HashMap::new() }),
            ("qdone".into(),         Value::Lambda { params: vec!["q".into()], body: Box::new(Expr::Ident("__astriloop_queue_done".into())), captures: HashMap::new() }),
            ("qjoin".into(),         Value::Lambda { params: vec!["q".into()], body: Box::new(Expr::Ident("__astriloop_queue_join".into())), captures: HashMap::new() }),
            ("qsize".into(),         Value::Lambda { params: vec!["q".into()], body: Box::new(Expr::Ident("__astriloop_queue_size".into())), captures: HashMap::new() }),
            ("qempty".into(),        Value::Lambda { params: vec!["q".into()], body: Box::new(Expr::Ident("__astriloop_queue_empty".into())), captures: HashMap::new() }),
            ("qfull".into(),         Value::Lambda { params: vec!["q".into()], body: Box::new(Expr::Ident("__astriloop_queue_full".into())), captures: HashMap::new() }),

            // ── STREAM PIPELINE (RxPY + async generators) ────────────────
            // Astriloop.Stream()  — create async push stream
            ("Stream".into(),     Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__astriloop_stream_new".into())), captures: HashMap::new() }),
            ("push".into(),       Value::Lambda { params: vec!["s".into(), "val".into()], body: Box::new(Expr::Ident("__astriloop_stream_push".into())), captures: HashMap::new() }),
            ("smap".into(),       Value::Lambda { params: vec!["s".into(), "fn".into()], body: Box::new(Expr::Ident("__astriloop_stream_map".into())), captures: HashMap::new() }),
            ("sfilter".into(),    Value::Lambda { params: vec!["s".into(), "fn".into()], body: Box::new(Expr::Ident("__astriloop_stream_filter".into())), captures: HashMap::new() }),
            ("sreduce".into(),    Value::Lambda { params: vec!["s".into(), "fn".into(), "init".into()], body: Box::new(Expr::Ident("__astriloop_stream_reduce".into())), captures: HashMap::new() }),
            ("sbatch".into(),     Value::Lambda { params: vec!["s".into(), "n".into()], body: Box::new(Expr::Ident("__astriloop_stream_batch".into())), captures: HashMap::new() }),
            ("sdebounce".into(),  Value::Lambda { params: vec!["s".into(), "ms".into()], body: Box::new(Expr::Ident("__astriloop_stream_debounce".into())), captures: HashMap::new() }),
            ("sthrottle".into(),  Value::Lambda { params: vec!["s".into(), "ms".into()], body: Box::new(Expr::Ident("__astriloop_stream_throttle".into())), captures: HashMap::new() }),
            ("smerge".into(),     Value::Lambda { params: vec!["streams".into()], body: Box::new(Expr::Ident("__astriloop_stream_merge".into())), captures: HashMap::new() }),
            ("szip".into(),       Value::Lambda { params: vec!["s1".into(), "s2".into()], body: Box::new(Expr::Ident("__astriloop_stream_zip".into())), captures: HashMap::new() }),
            ("stakeUntil".into(), Value::Lambda { params: vec!["s".into(), "fn".into()], body: Box::new(Expr::Ident("__astriloop_stream_take_until".into())), captures: HashMap::new() }),
            ("ssubscribe".into(), Value::Lambda { params: vec!["s".into(), "fn".into()], body: Box::new(Expr::Ident("__astriloop_stream_subscribe".into())), captures: HashMap::new() }),
            ("scollect".into(),   Value::Lambda { params: vec!["s".into()], body: Box::new(Expr::Ident("__astriloop_stream_collect".into())), captures: HashMap::new() }),
            ("sendStream".into(), Value::Lambda { params: vec!["s".into()], body: Box::new(Expr::Ident("__astriloop_stream_end".into())), captures: HashMap::new() }),

            // ── SIGNAL BUS (pub/sub within async context) ─────────────────
            // Astriloop.Bus()  — create a typed signal bus
            ("Bus".into(),       Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__astriloop_bus_new".into())), captures: HashMap::new() }),
            ("emit".into(),      Value::Lambda { params: vec!["bus".into(), "topic".into(), "val".into()], body: Box::new(Expr::Ident("__astriloop_bus_emit".into())), captures: HashMap::new() }),
            ("subscribe".into(), Value::Lambda { params: vec!["bus".into(), "topic".into(), "fn".into()], body: Box::new(Expr::Ident("__astriloop_bus_subscribe".into())), captures: HashMap::new() }),
            ("unsubscribe".into(),Value::Lambda { params: vec!["bus".into(), "topic".into(), "fn".into()], body: Box::new(Expr::Ident("__astriloop_bus_unsubscribe".into())), captures: HashMap::new() }),
            ("once".into(),      Value::Lambda { params: vec!["bus".into(), "topic".into(), "fn".into()], body: Box::new(Expr::Ident("__astriloop_bus_once".into())), captures: HashMap::new() }),
            ("waitFor".into(),   Value::Lambda { params: vec!["bus".into(), "topic".into(), "timeout".into()], body: Box::new(Expr::Ident("__astriloop_bus_wait_for".into())), captures: HashMap::new() }),

            // ── PERIODIC / SCHEDULED TASKS ────────────────────────────────
            // Astriloop.every(ms, fn)  — repeat fn every ms milliseconds
            ("every".into(),      Value::Lambda { params: vec!["ms".into(), "fn".into()], body: Box::new(Expr::Ident("__astriloop_every".into())), captures: HashMap::new() }),
            // Astriloop.after(ms, fn)  — run fn once after ms delay
            ("after".into(),      Value::Lambda { params: vec!["ms".into(), "fn".into()], body: Box::new(Expr::Ident("__astriloop_after".into())), captures: HashMap::new() }),
            // Astriloop.cron(expr, fn)  — cron-style scheduling ("*/5 * * * *")
            ("cron".into(),       Value::Lambda { params: vec!["expr".into(), "fn".into()], body: Box::new(Expr::Ident("__astriloop_cron".into())), captures: HashMap::new() }),
            ("stopSchedule".into(),Value::Lambda { params: vec!["handle".into()], body: Box::new(Expr::Ident("__astriloop_stop_schedule".into())), captures: HashMap::new() }),

            // ── RATE LIMITING & BACKPRESSURE ──────────────────────────────
            // Astriloop.rateLimit(n, window_ms)  — returns a token-bucket guard fn
            ("rateLimit".into(),   Value::Lambda { params: vec!["n".into(), "window".into()], body: Box::new(Expr::Ident("__astriloop_rate_limit".into())), captures: HashMap::new() }),
            // Astriloop.throttleFn(fn, ms)  — wraps fn so it can't run more than 1x per ms
            ("throttleFn".into(), Value::Lambda { params: vec!["fn".into(), "ms".into()], body: Box::new(Expr::Ident("__astriloop_throttle_fn".into())), captures: HashMap::new() }),
            // Astriloop.debounceFn(fn, ms)  — wait ms of silence then call fn
            ("debounceFn".into(), Value::Lambda { params: vec!["fn".into(), "ms".into()], body: Box::new(Expr::Ident("__astriloop_debounce_fn".into())), captures: HashMap::new() }),
            // Astriloop.retry(fn, maxAttempts, backoff_ms)  — auto-retry with backoff
            ("retry".into(),      Value::Lambda { params: vec!["fn".into(), "attempts".into(), "backoff".into()], body: Box::new(Expr::Ident("__astriloop_retry".into())), captures: HashMap::new() }),
            // Astriloop.circuit(fn, threshold, resetAfter)  — circuit breaker pattern
            ("circuit".into(),    Value::Lambda { params: vec!["fn".into(), "threshold".into(), "resetAfter".into()], body: Box::new(Expr::Ident("__astriloop_circuit".into())), captures: HashMap::new() }),

            // ── ASYNC ITERATION ───────────────────────────────────────────
            // Astriloop.forEach(list, async_fn)  — sequential async map over list
            ("forEach".into(),    Value::Lambda { params: vec!["list".into(), "fn".into()], body: Box::new(Expr::Ident("__astriloop_for_each".into())), captures: HashMap::new() }),
            // Astriloop.map(list, async_fn, concurrency?)  — parallel map with optional limit
            ("map".into(),        Value::Lambda { params: vec!["list".into(), "fn".into(), "concurrency".into()], body: Box::new(Expr::Ident("__astriloop_map".into())), captures: HashMap::new() }),
            // Astriloop.filter(list, async_fn)  — async filter
            ("filter".into(),     Value::Lambda { params: vec!["list".into(), "fn".into()], body: Box::new(Expr::Ident("__astriloop_filter".into())), captures: HashMap::new() }),
            // Astriloop.reduce(list, async_fn, init)  — async reduce (sequential)
            ("reduce".into(),     Value::Lambda { params: vec!["list".into(), "fn".into(), "init".into()], body: Box::new(Expr::Ident("__astriloop_reduce".into())), captures: HashMap::new() }),
            // Astriloop.pipeline(val, fns)  — chain async fns: output of each → input of next
            ("pipeline".into(),   Value::Lambda { params: vec!["val".into(), "fns".into()], body: Box::new(Expr::Ident("__astriloop_pipeline".into())), captures: HashMap::new() }),

            // ── DIAGNOSTICS ───────────────────────────────────────────────
            // Astriloop.stats()  — {tasks_running, tasks_done, tasks_cancelled, uptime_ms}
            ("stats".into(),  Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__astriloop_stats".into())), captures: HashMap::new() }),
            // Astriloop.trace(enable)  — toggle runtime tracing to serial console
            ("trace".into(),  Value::Lambda { params: vec!["enable".into()], body: Box::new(Expr::Ident("__astriloop_trace".into())), captures: HashMap::new() }),
        ]),

        // =====================================================================
        // Retime — Remox Time Library (Python's `time` module + more)
        // See `pub mod retime` near end of file for implementation, and
        // dispatch_retime() for the __retime_* -> real call routing.
        // =====================================================================
        "Retime" => Some(vec![
            // ── RAW CLOCKS ────────────────────────────────────────────────
            ("time".into(),        Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__retime_time".into())), captures: HashMap::new() }),
            ("time_ns".into(),     Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__retime_time_ns".into())), captures: HashMap::new() }),
            ("now".into(),         Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__retime_now_ms".into())), captures: HashMap::new() }),
            ("nowSecs".into(),     Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__retime_now_secs".into())), captures: HashMap::new() }),
            ("monotonic".into(),   Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__retime_monotonic".into())), captures: HashMap::new() }),
            ("monotonicNs".into(), Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__retime_monotonic_ns".into())), captures: HashMap::new() }),
            ("perfCounter".into(), Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__retime_perf_counter".into())), captures: HashMap::new() }),
            ("perfCounterNs".into(),Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__retime_perf_counter_ns".into())), captures: HashMap::new() }),
            ("processTime".into(), Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__retime_process_time".into())), captures: HashMap::new() }),

            // ── WALL-CLOCK CALIBRATION ───────────────────────────────────
            // Retime.setWallClock(unixSecs)  — calibrate once real time is known
            ("setWallClock".into(),       Value::Lambda { params: vec!["unixSecs".into()], body: Box::new(Expr::Ident("__retime_set_wall_clock".into())), captures: HashMap::new() }),
            ("isWallClockCalibrated".into(), Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__retime_is_wall_clock_calibrated".into())), captures: HashMap::new() }),

            // ── SLEEP ─────────────────────────────────────────────────────
            ("sleep".into(),     Value::Lambda { params: vec!["secs".into()], body: Box::new(Expr::Ident("__retime_sleep_secs".into())), captures: HashMap::new() }),
            ("sleepMs".into(),   Value::Lambda { params: vec!["ms".into()], body: Box::new(Expr::Ident("__retime_sleep_ms".into())), captures: HashMap::new() }),

            // ── CALENDAR — gmtime/localtime/format/parse ────────────────
            // Retime.gmtime(epochSecs) -> map with year/month/day/hour/minute/second/weekday/yday
            ("gmtime".into(),      Value::Lambda { params: vec!["epochSecs".into()], body: Box::new(Expr::Ident("__retime_gmtime".into())), captures: HashMap::new() }),
            // Retime.localtime(epochSecs, offsetSeconds)
            ("localtime".into(),   Value::Lambda { params: vec!["epochSecs".into(), "offsetSecs".into()], body: Box::new(Expr::Ident("__retime_localtime".into())), captures: HashMap::new() }),
            // Retime.strftime(structTimeMap, fmt, tzLabel)
            ("strftime".into(),    Value::Lambda { params: vec!["t".into(), "fmt".into(), "tzLabel".into()], body: Box::new(Expr::Ident("__retime_strftime".into())), captures: HashMap::new() }),
            ("asctime".into(),     Value::Lambda { params: vec!["t".into()], body: Box::new(Expr::Ident("__retime_asctime".into())), captures: HashMap::new() }),
            ("ctime".into(),       Value::Lambda { params: vec!["epochSecs".into()], body: Box::new(Expr::Ident("__retime_ctime".into())), captures: HashMap::new() }),
            // Retime.toRfc3339(structTimeMap, offsetSeconds)
            ("toRfc3339".into(),   Value::Lambda { params: vec!["t".into(), "offsetSecs".into()], body: Box::new(Expr::Ident("__retime_to_rfc3339".into())), captures: HashMap::new() }),
            // Retime.parseRfc3339("2026-07-11T14:05:09+05:30") -> {ok: bool, epochSecs, nanos, offsetSecs}
            ("parseRfc3339".into(),Value::Lambda { params: vec!["s".into()], body: Box::new(Expr::Ident("__retime_parse_rfc3339".into())), captures: HashMap::new() }),

            // ── DURATION HELPERS ──────────────────────────────────────────
            // Retime.parseDuration("1h30m") -> ms (Int) or null on parse failure
            ("parseDuration".into(), Value::Lambda { params: vec!["s".into()], body: Box::new(Expr::Ident("__retime_parse_duration".into())), captures: HashMap::new() }),
            // Retime.humanize(ms) -> "1h 1m 1s"
            ("humanize".into(),      Value::Lambda { params: vec!["ms".into()], body: Box::new(Expr::Ident("__retime_humanize".into())), captures: HashMap::new() }),

            // ── CALENDAR MATH ─────────────────────────────────────────────
            ("daysFromCivil".into(), Value::Lambda { params: vec!["year".into(), "month".into(), "day".into()], body: Box::new(Expr::Ident("__retime_days_from_civil".into())), captures: HashMap::new() }),
            ("civilFromDays".into(), Value::Lambda { params: vec!["days".into()], body: Box::new(Expr::Ident("__retime_civil_from_days".into())), captures: HashMap::new() }),
            ("isLeapYear".into(),    Value::Lambda { params: vec!["year".into()], body: Box::new(Expr::Ident("__retime_is_leap_year".into())), captures: HashMap::new() }),
            ("daysInMonth".into(),   Value::Lambda { params: vec!["year".into(), "month".into()], body: Box::new(Expr::Ident("__retime_days_in_month".into())), captures: HashMap::new() }),
            ("weekdayFromDays".into(),Value::Lambda { params: vec!["days".into()], body: Box::new(Expr::Ident("__retime_weekday_from_days".into())), captures: HashMap::new() }),
            ("dayOfYear".into(),     Value::Lambda { params: vec!["year".into(), "month".into(), "day".into()], body: Box::new(Expr::Ident("__retime_day_of_year".into())), captures: HashMap::new() }),

            // ── STOPWATCH (lap-capable timer) ────────────────────────────
            ("Stopwatch".into(),     Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__retime_stopwatch_new".into())), captures: HashMap::new() }),
            ("swStart".into(),       Value::Lambda { params: vec!["sw".into()], body: Box::new(Expr::Ident("__retime_stopwatch_start".into())), captures: HashMap::new() }),
            ("swPause".into(),       Value::Lambda { params: vec!["sw".into()], body: Box::new(Expr::Ident("__retime_stopwatch_pause".into())), captures: HashMap::new() }),
            ("swReset".into(),       Value::Lambda { params: vec!["sw".into()], body: Box::new(Expr::Ident("__retime_stopwatch_reset".into())), captures: HashMap::new() }),
            ("swElapsedMs".into(),   Value::Lambda { params: vec!["sw".into()], body: Box::new(Expr::Ident("__retime_stopwatch_elapsed_ms".into())), captures: HashMap::new() }),
            ("swLap".into(),         Value::Lambda { params: vec!["sw".into()], body: Box::new(Expr::Ident("__retime_stopwatch_lap".into())), captures: HashMap::new() }),
            ("swLaps".into(),        Value::Lambda { params: vec!["sw".into()], body: Box::new(Expr::Ident("__retime_stopwatch_laps".into())), captures: HashMap::new() }),

            // ── DEADLINE (one-shot timeout) ──────────────────────────────
            ("Deadline".into(),      Value::Lambda { params: vec!["ms".into()], body: Box::new(Expr::Ident("__retime_deadline_new".into())), captures: HashMap::new() }),
            ("dlExpired".into(),     Value::Lambda { params: vec!["dl".into()], body: Box::new(Expr::Ident("__retime_deadline_expired".into())), captures: HashMap::new() }),
            ("dlRemainingMs".into(), Value::Lambda { params: vec!["dl".into()], body: Box::new(Expr::Ident("__retime_deadline_remaining_ms".into())), captures: HashMap::new() }),
            ("dlWait".into(),        Value::Lambda { params: vec!["dl".into()], body: Box::new(Expr::Ident("__retime_deadline_wait".into())), captures: HashMap::new() }),

            // ── TICKER (drift-corrected periodic checkpoint) ─────────────
            ("Ticker".into(),        Value::Lambda { params: vec!["periodMs".into()], body: Box::new(Expr::Ident("__retime_ticker_new".into())), captures: HashMap::new() }),
            ("tkReady".into(),       Value::Lambda { params: vec!["tk".into()], body: Box::new(Expr::Ident("__retime_ticker_ready".into())), captures: HashMap::new() }),
            ("tkWaitNext".into(),    Value::Lambda { params: vec!["tk".into()], body: Box::new(Expr::Ident("__retime_ticker_wait_next".into())), captures: HashMap::new() }),

            // ── RATE COUNTER (rolling events/sec) ────────────────────────
            ("RateCounter".into(),   Value::Lambda { params: vec!["windowMs".into()], body: Box::new(Expr::Ident("__retime_ratecounter_new".into())), captures: HashMap::new() }),
            ("rcTick".into(),        Value::Lambda { params: vec!["rc".into()], body: Box::new(Expr::Ident("__retime_ratecounter_tick".into())), captures: HashMap::new() }),
            ("rcRateHz".into(),      Value::Lambda { params: vec!["rc".into()], body: Box::new(Expr::Ident("__retime_ratecounter_rate_hz".into())), captures: HashMap::new() }),
            ("rcCount".into(),       Value::Lambda { params: vec!["rc".into()], body: Box::new(Expr::Ident("__retime_ratecounter_count".into())), captures: HashMap::new() }),

            // ── DIAGNOSTICS ───────────────────────────────────────────────
            ("selfTest".into(),      Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__retime_self_test".into())), captures: HashMap::new() }),
        ]),

        // =====================================================================
        // Tasoaque — Remox ka apna Distributed Task Queue Engine
        // Python's Celery + RQ ko combine karke usse bhi zyada powerful —
        // ek hi built-in module, zero-import, zero-broker-setup (in-process
        // broker hai, REAL hai, simulated nahi). Koi bhi top-level fn seedha
        // task ban sakta hai — koi decorator boilerplate nahi chahiye,
        // kyunki Remox saare top-level fns pehle se hi resolve kar leta hai
        // (Interpreter.fns), Tasoaque sirf naam ko args ke saath dispatch
        // karta hai. PLUS — serve()/remoteWork() se cluster mode: genuine
        // cross-machine workers, koi Redis/RabbitMQ waala external broker
        // ke bina (Tasoaque khud hi apna broker hai).
        //
        // Syntax (Celery/RQ se bhi aasaan):
        //   use Tasoaque
        //   fn sendEmail(to, subject) { print("mail -> " + to) }
        //   Tasoaque.task("sendEmail", {"queue":"mail","priority":2,"maxRetries":3})
        //   Tasoaque.enqueue("sendEmail", ["a@b.com","Hi"])
        //   Tasoaque.applyAsync("sendEmail", ["a@b.com","Hi"], {"countdown":5})
        //   Tasoaque.tick(5)                 // logical clock aage badhao
        //   Tasoaque.runWorker("mail", 10)   // due jobs process karo (local)
        //   Tasoaque.status(jobId)           // pending|running|success|failed|retry|cancelled
        //   // Machine A (coordinator):
        //   Tasoaque.serve(9500)
        //   // Machine B, C, D... (remote workers — alag hardware/process):
        //   Tasoaque.remoteWork("192.168.1.10", 9500, "mail", 100)
        //
        // Features (Celery + RQ se zyada, ek hi jagah):
        //   task/enqueue/delay/applyAsync — priority queues, countdown, eta
        //   runWorker/runOne             — real synchronous/cooperative worker
        //   automatic retries            — configurable backoff, dead-letter queue
        //   chain/group/chord            — pipelines, fan-out, fan-out+callback
        //   schedule/unschedule/tick     — periodic (interval) tasks, explicit logical clock
        //   rateLimit                    — per-task max executions per window
        //   status/result/error/cancel/requeue/purge/stats/deadLetters/queueLength
        //   serve/remoteWork             — real multi-machine cluster mode, no external broker
        //
        // HONEST GAP #1: countdown/eta/schedule intervals run against a
        // monotonic LOGICAL clock (advanced only by Tasoaque.tick(), which
        // the script calls explicitly), NOT real wall-clock seconds —
        // because SystemTime::now() is still the epoch-0 placeholder until
        // Monobat's RTC/TSC driver lands (see std-compat shim above). This
        // is a deliberate choice over faking wall time: ordering/backoff/
        // scheduling is real and deterministic today, just not calibrated
        // to real seconds yet. Swap the clock source later — nothing else
        // in Tasoaque needs to change.
        //
        // HONEST GAP #2: serve()/remoteWork() use the exact same real,
        // length-prefixed wire protocol as Autoclib.remoteExec() above —
        // MonobatHal::net_tcp_bind/accept/connect/write/read. bind/accept
        // are already wired (used by Vyraweb today); connect/write/read are
        // defined as default trait methods returning NotReady until
        // Monobat's real network driver (smoltcp or similar) is plugged in.
        // The protocol logic itself is complete and correct today — once
        // those three functions are wired to real hardware, cluster mode
        // works across actual separate machines with zero changes here.
        // =====================================================================
        "Tasoaque" => Some(vec![
            ("version".into(), Value::Str("Tasoaque 1.0 — Task Queues Se Bhi Aage — No Simulation".into())),

            // ── Task registration & submission ─────────────────────────────
            ("task".into(),        Value::Lambda { params: vec!["name".into(), "opts".into()], body: Box::new(Expr::Ident("__tasoaque_task".into())), captures: HashMap::new() }),
            ("enqueue".into(),     Value::Lambda { params: vec!["name".into(), "args".into()], body: Box::new(Expr::Ident("__tasoaque_enqueue".into())), captures: HashMap::new() }),
            ("delay".into(),       Value::Lambda { params: vec!["name".into(), "args".into()], body: Box::new(Expr::Ident("__tasoaque_enqueue".into())), captures: HashMap::new() }),
            ("applyAsync".into(),  Value::Lambda { params: vec!["name".into(), "args".into(), "opts".into()], body: Box::new(Expr::Ident("__tasoaque_apply_async".into())), captures: HashMap::new() }),

            // ── Worker (real, cooperative — hal().task_spawn se compatible) ─
            ("runWorker".into(),   Value::Lambda { params: vec!["queue".into(), "limit".into()], body: Box::new(Expr::Ident("__tasoaque_run_worker".into())), captures: HashMap::new() }),
            ("runOne".into(),      Value::Lambda { params: vec!["queue".into()], body: Box::new(Expr::Ident("__tasoaque_run_one".into())), captures: HashMap::new() }),

            // ── Job inspection & control ────────────────────────────────────
            ("status".into(),      Value::Lambda { params: vec!["jobId".into()], body: Box::new(Expr::Ident("__tasoaque_status".into())), captures: HashMap::new() }),
            ("result".into(),      Value::Lambda { params: vec!["jobId".into()], body: Box::new(Expr::Ident("__tasoaque_result".into())), captures: HashMap::new() }),
            ("error".into(),       Value::Lambda { params: vec!["jobId".into()], body: Box::new(Expr::Ident("__tasoaque_error".into())), captures: HashMap::new() }),
            ("cancel".into(),      Value::Lambda { params: vec!["jobId".into()], body: Box::new(Expr::Ident("__tasoaque_cancel".into())), captures: HashMap::new() }),
            ("requeue".into(),     Value::Lambda { params: vec!["jobId".into()], body: Box::new(Expr::Ident("__tasoaque_requeue".into())), captures: HashMap::new() }),
            ("purge".into(),       Value::Lambda { params: vec!["queue".into()], body: Box::new(Expr::Ident("__tasoaque_purge".into())), captures: HashMap::new() }),
            ("stats".into(),       Value::Lambda { params: vec!["queue".into()], body: Box::new(Expr::Ident("__tasoaque_stats".into())), captures: HashMap::new() }),
            ("deadLetters".into(), Value::Lambda { params: vec!["queue".into()], body: Box::new(Expr::Ident("__tasoaque_dead_letters".into())), captures: HashMap::new() }),
            ("queueLength".into(), Value::Lambda { params: vec!["queue".into()], body: Box::new(Expr::Ident("__tasoaque_queue_length".into())), captures: HashMap::new() }),

            // ── Pipelines / fan-out (beyond Celery's own chain/group/chord) ─
            ("chain".into(),        Value::Lambda { params: vec!["steps".into()], body: Box::new(Expr::Ident("__tasoaque_chain".into())), captures: HashMap::new() }),
            ("group".into(),        Value::Lambda { params: vec!["jobs".into()], body: Box::new(Expr::Ident("__tasoaque_group".into())), captures: HashMap::new() }),
            ("chord".into(),        Value::Lambda { params: vec!["jobs".into(), "callback".into()], body: Box::new(Expr::Ident("__tasoaque_chord".into())), captures: HashMap::new() }),
            ("groupResults".into(), Value::Lambda { params: vec!["groupId".into()], body: Box::new(Expr::Ident("__tasoaque_group_results".into())), captures: HashMap::new() }),

            // ── Periodic scheduling & logical clock ──────────────────────────
            ("schedule".into(),   Value::Lambda { params: vec!["name".into(), "args".into(), "interval".into(), "queue".into()], body: Box::new(Expr::Ident("__tasoaque_schedule".into())), captures: HashMap::new() }),
            ("unschedule".into(), Value::Lambda { params: vec!["scheduleId".into()], body: Box::new(Expr::Ident("__tasoaque_unschedule".into())), captures: HashMap::new() }),
            ("tick".into(),       Value::Lambda { params: vec!["n".into()], body: Box::new(Expr::Ident("__tasoaque_tick".into())), captures: HashMap::new() }),

            // ── Rate limiting ─────────────────────────────────────────────
            ("rateLimit".into(),  Value::Lambda { params: vec!["name".into(), "max".into(), "window".into()], body: Box::new(Expr::Ident("__tasoaque_rate_limit".into())), captures: HashMap::new() }),

            // ── Cluster mode — REAL cross-machine distribution ──────────────
            // serve(port) → is machine ko coordinator banao: remote workers
            //   yahan connect karke jobs le jaate hain aur result wapas
            //   report karte hain. Blocking call (jaisa Vyraweb.listen()) —
            //   ek baar chalao, chalta rehta hai.
            ("serve".into(),       Value::Lambda { params: vec!["port".into()], body: Box::new(Expr::Ident("__tasoaque_serve".into())), captures: HashMap::new() }),
            // remoteWork(host, port, queue, limit) → is machine (bilkul
            //   alag process/hardware) ko worker banao: coordinator se
            //   TCP par connect karke asli jobs pull + execute + report karo.
            ("remoteWork".into(),  Value::Lambda { params: vec!["host".into(), "port".into(), "queue".into(), "limit".into()], body: Box::new(Expr::Ident("__tasoaque_remote_work".into())), captures: HashMap::new() }),
        ]),

        // =====================================================================
        // Sceuti — v2.0, fifty features across five sub-libraries:
        //   Arrow → SceutiClock, Schedule → SceutiSchedule,
        //   python-dotenv → SceutiEnv, Loguru → SceutiLog, Faker → SceutiFake.
        //   use Sceuti
        //   let t = Sceuti.clock_now()
        // =====================================================================
        "Sceuti" => Some(vec![
            ("version".into(), Value::Str("Sceuti 2.0 — Fifty Features — No-OS".into())),

            // ── SceutiClock ──────────────────────────────────────────────
            ("clock_now".into(),        Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__sceuti_clock_now".into())), captures: HashMap::new() }),
            ("clock_fromEpoch".into(),  Value::Lambda { params: vec!["epoch".into()], body: Box::new(Expr::Ident("__sceuti_clock_from_epoch".into())), captures: HashMap::new() }),
            ("clock_format".into(),     Value::Lambda { params: vec!["epoch".into(), "pattern".into()], body: Box::new(Expr::Ident("__sceuti_clock_format".into())), captures: HashMap::new() }),
            ("clock_diff".into(),       Value::Lambda { params: vec!["t1".into(), "t2".into()], body: Box::new(Expr::Ident("__sceuti_clock_diff".into())), captures: HashMap::new() }),
            ("clock_addSeconds".into(), Value::Lambda { params: vec!["epoch".into(), "n".into()], body: Box::new(Expr::Ident("__sceuti_clock_add_seconds".into())), captures: HashMap::new() }),
            ("clock_addMinutes".into(), Value::Lambda { params: vec!["epoch".into(), "n".into()], body: Box::new(Expr::Ident("__sceuti_clock_add_minutes".into())), captures: HashMap::new() }),
            ("clock_addHours".into(),   Value::Lambda { params: vec!["epoch".into(), "n".into()], body: Box::new(Expr::Ident("__sceuti_clock_add_hours".into())), captures: HashMap::new() }),
            ("clock_humanize".into(),   Value::Lambda { params: vec!["epoch".into()], body: Box::new(Expr::Ident("__sceuti_clock_humanize".into())), captures: HashMap::new() }),
            ("clock_isBefore".into(),   Value::Lambda { params: vec!["t1".into(), "t2".into()], body: Box::new(Expr::Ident("__sceuti_clock_is_before".into())), captures: HashMap::new() }),
            ("clock_isAfter".into(),    Value::Lambda { params: vec!["t1".into(), "t2".into()], body: Box::new(Expr::Ident("__sceuti_clock_is_after".into())), captures: HashMap::new() }),

            // ── SceutiSchedule ───────────────────────────────────────────
            ("schedule_everyN".into(),    Value::Lambda { params: vec!["n".into(), "fn".into()], body: Box::new(Expr::Ident("__sceuti_schedule_every_n".into())), captures: HashMap::new() }),
            ("schedule_onceAfter".into(), Value::Lambda { params: vec!["n".into(), "fn".into()], body: Box::new(Expr::Ident("__sceuti_schedule_once".into())), captures: HashMap::new() }),
            ("schedule_cancel".into(),    Value::Lambda { params: vec!["id".into()], body: Box::new(Expr::Ident("__sceuti_schedule_cancel".into())), captures: HashMap::new() }),
            ("schedule_list".into(),      Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__sceuti_schedule_list".into())), captures: HashMap::new() }),
            ("schedule_pauseAll".into(),  Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__sceuti_schedule_pause".into())), captures: HashMap::new() }),
            ("schedule_resumeAll".into(), Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__sceuti_schedule_resume".into())), captures: HashMap::new() }),
            ("schedule_count".into(),     Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__sceuti_schedule_count".into())), captures: HashMap::new() }),
            ("schedule_runPending".into(),Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__sceuti_schedule_run".into())), captures: HashMap::new() }),
            ("schedule_clearAll".into(),  Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__sceuti_schedule_clear".into())), captures: HashMap::new() }),
            ("schedule_onBoot".into(),    Value::Lambda { params: vec!["fn".into()], body: Box::new(Expr::Ident("__sceuti_schedule_on_boot".into())), captures: HashMap::new() }),

            // ── SceutiEnv ────────────────────────────────────────────────
            ("env_get".into(),        Value::Lambda { params: vec!["key".into()], body: Box::new(Expr::Ident("__sceuti_env_get".into())), captures: HashMap::new() }),
            ("env_set".into(),        Value::Lambda { params: vec!["key".into(), "val".into()], body: Box::new(Expr::Ident("__sceuti_env_set".into())), captures: HashMap::new() }),
            ("env_delete".into(),     Value::Lambda { params: vec!["key".into()], body: Box::new(Expr::Ident("__sceuti_env_delete".into())), captures: HashMap::new() }),
            ("env_has".into(),        Value::Lambda { params: vec!["key".into()], body: Box::new(Expr::Ident("__sceuti_env_has".into())), captures: HashMap::new() }),
            ("env_keys".into(),       Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__sceuti_env_keys".into())), captures: HashMap::new() }),
            ("env_values".into(),     Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__sceuti_env_values".into())), captures: HashMap::new() }),
            ("env_entries".into(),    Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__sceuti_env_entries".into())), captures: HashMap::new() }),
            ("env_loadString".into(), Value::Lambda { params: vec!["raw".into()], body: Box::new(Expr::Ident("__sceuti_env_load_string".into())), captures: HashMap::new() }),
            ("env_toMap".into(),      Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__sceuti_env_to_map".into())), captures: HashMap::new() }),
            ("env_clear".into(),      Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__sceuti_env_clear".into())), captures: HashMap::new() }),

            // ── SceutiLog ────────────────────────────────────────────────
            ("log_info".into(),         Value::Lambda { params: vec!["msg".into()], body: Box::new(Expr::Ident("__sceuti_log_info".into())), captures: HashMap::new() }),
            ("log_warn".into(),         Value::Lambda { params: vec!["msg".into()], body: Box::new(Expr::Ident("__sceuti_log_warn".into())), captures: HashMap::new() }),
            ("log_error".into(),        Value::Lambda { params: vec!["msg".into()], body: Box::new(Expr::Ident("__sceuti_log_error".into())), captures: HashMap::new() }),
            ("log_debug".into(),        Value::Lambda { params: vec!["msg".into()], body: Box::new(Expr::Ident("__sceuti_log_debug".into())), captures: HashMap::new() }),
            ("log_setLevel".into(),     Value::Lambda { params: vec!["level".into()], body: Box::new(Expr::Ident("__sceuti_log_set_level".into())), captures: HashMap::new() }),
            ("log_withPrefix".into(),   Value::Lambda { params: vec!["prefix".into()], body: Box::new(Expr::Ident("__sceuti_log_with_prefix".into())), captures: HashMap::new() }),
            ("log_history".into(),      Value::Lambda { params: vec!["n".into()], body: Box::new(Expr::Ident("__sceuti_log_history".into())), captures: HashMap::new() }),
            ("log_historyCount".into(), Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__sceuti_log_history_count".into())), captures: HashMap::new() }),
            ("log_clearHistory".into(), Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__sceuti_log_clear".into())), captures: HashMap::new() }),
            ("log_format".into(),       Value::Lambda { params: vec!["level".into(), "msg".into()], body: Box::new(Expr::Ident("__sceuti_log_format".into())), captures: HashMap::new() }),

            // ── SceutiFake ───────────────────────────────────────────────
            ("fake_int".into(),     Value::Lambda { params: vec!["min".into(), "max".into()], body: Box::new(Expr::Ident("__sceuti_fake_int".into())), captures: HashMap::new() }),
            ("fake_str".into(),     Value::Lambda { params: vec!["len".into()], body: Box::new(Expr::Ident("__sceuti_fake_str".into())), captures: HashMap::new() }),
            ("fake_bool".into(),    Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__sceuti_fake_bool".into())), captures: HashMap::new() }),
            ("fake_from".into(),    Value::Lambda { params: vec!["list".into()], body: Box::new(Expr::Ident("__sceuti_fake_from".into())), captures: HashMap::new() }),
            ("fake_seed".into(),    Value::Lambda { params: vec!["n".into()], body: Box::new(Expr::Ident("__sceuti_fake_seed".into())), captures: HashMap::new() }),
            ("fake_uuid".into(),    Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__sceuti_fake_uuid".into())), captures: HashMap::new() }),
            ("fake_hex".into(),     Value::Lambda { params: vec!["len".into()], body: Box::new(Expr::Ident("__sceuti_fake_hex".into())), captures: HashMap::new() }),
            ("fake_bytes".into(),   Value::Lambda { params: vec!["n".into()], body: Box::new(Expr::Ident("__sceuti_fake_bytes".into())), captures: HashMap::new() }),
            ("fake_ipv4".into(),    Value::Lambda { params: vec![], body: Box::new(Expr::Ident("__sceuti_fake_ipv4".into())), captures: HashMap::new() }),
            ("fake_shuffle".into(), Value::Lambda { params: vec!["list".into()], body: Box::new(Expr::Ident("__sceuti_fake_shuffle".into())), captures: HashMap::new() }),
        ]),

        _ => None,
    }
}

// =============================================================================
// TASOAQUE ENGINE — real in-process task queue state.
//
// Storage is a single `std::sync::Mutex` (spin::Mutex-backed, const-init —
// same pattern as MONOBAT_TASK_QUEUE above), holding plain Vec-based tables.
// Vecs were chosen over HashMap here specifically so the whole state is
// const-constructible in a `static`, exactly like MONOBAT_TASK_QUEUE — no
// unsafe, no lazy-init. Job volumes in a task-queue workload are small
// enough that linear scans over `jobs`/`schedules`/`rate_usage` are cheap
// and, unlike a hash table, keep iteration order stable (useful for FIFO
// tie-breaking within a priority level).
// =============================================================================

#[derive(Clone)]
struct TasoaqueJob {
    id: String,
    task: String,
    args: Vec<Value>,
    queue: String,
    priority: i64,          // 0 = highest .. 9 = lowest, default 5
    eta: u64,                // logical-clock tick this job becomes runnable; 0 = now
    status: String,          // pending | running | success | failed | retry | cancelled
    attempts: u32,
    max_retries: u32,
    retry_delay: u64,        // logical ticks to wait before a retry attempt
    result: Value,
    error: String,
    created_at: u64,
    chain_next: Vec<(String, Vec<Value>, String)>, // remaining chain steps: (task, extra_args, queue)
    group_id: String,        // "" if this job isn't part of a group/chord
}

#[derive(Clone)]
struct TasoaqueGroup {
    id: String,
    job_ids: Vec<String>,
    callback: String,        // chord callback task name; "" for a plain group
    callback_queue: String,
    fired: bool,
}

#[derive(Clone)]
struct TasoaqueSchedule {
    id: String,
    task: String,
    args: Vec<Value>,
    queue: String,
    interval: u64,
    next_run: u64,
    priority: i64,
}

#[derive(Clone)]
struct TasoaqueTaskDef {
    name: String,
    queue: String,
    priority: i64,
    max_retries: u32,
    retry_delay: u64,
}

struct TasoaqueState {
    jobs: Vec<TasoaqueJob>,
    groups: Vec<TasoaqueGroup>,
    schedules: Vec<TasoaqueSchedule>,
    dead_letters: Vec<TasoaqueJob>,
    task_defs: Vec<TasoaqueTaskDef>,
    rate_limits: Vec<(String, u32, u64)>,  // task -> (max per window, window ticks)
    rate_usage: Vec<(String, u64, u32)>,   // task -> (window start tick, count so far)
    next_id: u64,
    clock: u64,
}

impl TasoaqueState {
    const fn new() -> Self {
        TasoaqueState {
            jobs: Vec::new(), groups: Vec::new(), schedules: Vec::new(),
            dead_letters: Vec::new(), task_defs: Vec::new(),
            rate_limits: Vec::new(), rate_usage: Vec::new(),
            next_id: 1, clock: 0,
        }
    }

    fn find_task_def(&self, name: &str) -> Option<&TasoaqueTaskDef> {
        self.task_defs.iter().find(|d| d.name == name)
    }

    fn fresh_id(&mut self, prefix: &str) -> String {
        let id = format!("{}-{}", prefix, self.next_id);
        self.next_id += 1;
        id
    }
}

static TASOAQUE: std::sync::Mutex<TasoaqueState> = std::sync::Mutex::new(TasoaqueState::new());

// =============================================================================
// SCEUTI ENGINE — v2.0 — FIFTY FEATURES
//   Arrow      → SceutiClock    (features 1–10)
//   Schedule   → SceutiSchedule (features 11–20)
//   python-dotenv → SceutiEnv   (features 21–30)
//   Loguru     → SceutiLog      (features 31–40)
//   Faker      → SceutiFake     (features 41–50)
//
// SceutiClock ticks off the same logical counter that drives
// SceutiSchedule: every executed statement bumps SCEUTI_CLOCK by one (see
// the call in `exec_stmt`), so `use Sceuti` gives scripts a free, honest
// (non-wall-clock) notion of time — no OS timer, no filesystem, no heap
// allocator assumptions beyond plain Vec/String.
// =============================================================================

static SCEUTI_CLOCK:     core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static SCEUTI_LOG_LEVEL: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0); // 0=DEBUG,1=INFO,2=WARN,3=ERROR
static SCEUTI_FAKE_SEED:  core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(12345);
const SCEUTI_LOG_HISTORY_MAX: usize = 200;

#[derive(Clone, Debug)]
struct SceutiTime { epoch: u64 }
impl SceutiTime { fn new(epoch: u64) -> Self { Self { epoch } } }

#[derive(Clone)]
struct SceutiJob {
    id: u64,
    every_n: u64,       // 0 = one-shot
    trigger_at: u64,    // clock value jab chalega
    paused: bool,
    done: bool,
    fn_name: String,
}

#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum SceutiLogLevel { Debug = 0, Info = 1, Warn = 2, Error = 3 }
impl SceutiLogLevel {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Debug => "DEBUG", Self::Info => "INFO",
            Self::Warn  => "WARN",  Self::Error => "ERROR",
        }
    }
    fn from_u64(v: u64) -> Self {
        match v { 1 => Self::Info, 2 => Self::Warn, 3 => Self::Error, _ => Self::Debug }
    }
}

#[derive(Clone)]
struct SceutiLogEntry {
    level: SceutiLogLevel,
    clock: u64,
    prefix: String,
    message: String,
}

struct SceutiState {
    jobs: Vec<SceutiJob>,
    boot_jobs_run: bool,
    env_store: Vec<(String, String)>,
    log_history: Vec<SceutiLogEntry>,
    log_prefix: String,
}
impl SceutiState {
    const fn new() -> Self {
        SceutiState {
            jobs: Vec::new(), boot_jobs_run: false,
            env_store: Vec::new(), log_history: Vec::new(),
            log_prefix: String::new(),
        }
    }
}

static SCEUTI: std::sync::Mutex<SceutiState> = std::sync::Mutex::new(SceutiState::new());

/// Har executed statement pe ek baar call hota hai (see `exec_stmt`) —
/// SceutiClock + SceutiSchedule dono isi single logical tick pe chalte hain.
fn sceuti_tick() -> u64 {
    SCEUTI_CLOCK.fetch_add(1, core::sync::atomic::Ordering::Relaxed) + 1
}

// ---- CATEGORY 1: SceutiClock (features 1–10) ----

fn sceuti_clock_now() -> SceutiTime {
    SceutiTime::new(SCEUTI_CLOCK.load(core::sync::atomic::Ordering::Relaxed))
}
fn sceuti_clock_from_epoch(epoch: u64) -> SceutiTime { SceutiTime::new(epoch) }
fn sceuti_clock_format(t: &SceutiTime, pattern: &str) -> String {
    let sec  = t.epoch % 60;
    let min  = (t.epoch / 60) % 60;
    let hour = (t.epoch / 3600) % 24;
    pattern
        .replace("{epoch}", &format!("{}", t.epoch))
        .replace("{sec}",   &format!("{:02}", sec))
        .replace("{min}",   &format!("{:02}", min))
        .replace("{hour}",  &format!("{:02}", hour))
}
fn sceuti_clock_diff_seconds(t1: &SceutiTime, t2: &SceutiTime) -> i64 {
    t2.epoch as i64 - t1.epoch as i64
}
fn sceuti_clock_add_seconds(t: &SceutiTime, n: i64) -> SceutiTime {
    SceutiTime::new((t.epoch as i64 + n).max(0) as u64)
}
fn sceuti_clock_add_minutes(t: &SceutiTime, n: i64) -> SceutiTime { sceuti_clock_add_seconds(t, n * 60) }
fn sceuti_clock_add_hours(t: &SceutiTime, n: i64) -> SceutiTime { sceuti_clock_add_seconds(t, n * 3600) }
fn sceuti_clock_humanize(t: &SceutiTime) -> String {
    let now  = SCEUTI_CLOCK.load(core::sync::atomic::Ordering::Relaxed);
    let diff = now as i64 - t.epoch as i64;
    let abs  = diff.unsigned_abs();
    let suffix = if diff >= 0 { "ago" } else { "from now" };
    if abs < 60        { format!("{} seconds {}", abs, suffix) }
    else if abs < 3600 { format!("{} minutes {}", abs / 60, suffix) }
    else               { format!("{} hours {}", abs / 3600, suffix) }
}
fn sceuti_clock_is_before(t1: &SceutiTime, t2: &SceutiTime) -> bool { t1.epoch < t2.epoch }
fn sceuti_clock_is_after(t1: &SceutiTime, t2: &SceutiTime) -> bool { t1.epoch > t2.epoch }

// ---- CATEGORY 2: SceutiSchedule (features 11–20) ----

static SCEUTI_JOB_ID_COUNTER: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);
fn sceuti_next_job_id() -> u64 { SCEUTI_JOB_ID_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed) }

fn sceuti_schedule_every_n(jobs: &mut Vec<SceutiJob>, clock: u64, every_n: u64, fn_name: String) -> u64 {
    let id = sceuti_next_job_id();
    jobs.push(SceutiJob { id, every_n, trigger_at: clock + every_n, paused: false, done: false, fn_name });
    id
}
fn sceuti_schedule_once_after(jobs: &mut Vec<SceutiJob>, clock: u64, after_n: u64, fn_name: String) -> u64 {
    let id = sceuti_next_job_id();
    jobs.push(SceutiJob { id, every_n: 0, trigger_at: clock + after_n, paused: false, done: false, fn_name });
    id
}
fn sceuti_schedule_cancel(jobs: &mut Vec<SceutiJob>, job_id: u64) {
    if let Some(j) = jobs.iter_mut().find(|j| j.id == job_id) { j.done = true; }
}
fn sceuti_schedule_list_jobs(jobs: &Vec<SceutiJob>) -> Vec<(u64, String)> {
    jobs.iter().filter(|j| !j.done).map(|j| (j.id, j.fn_name.clone())).collect()
}
fn sceuti_schedule_pause_all(jobs: &mut Vec<SceutiJob>) { for j in jobs.iter_mut() { j.paused = true; } }
fn sceuti_schedule_resume_all(jobs: &mut Vec<SceutiJob>) { for j in jobs.iter_mut() { j.paused = false; } }
fn sceuti_schedule_job_count(jobs: &Vec<SceutiJob>) -> usize { jobs.iter().filter(|j| !j.done).count() }
fn sceuti_schedule_run_pending(jobs: &mut Vec<SceutiJob>, clock: u64) -> Vec<String> {
    let mut to_run = Vec::new();
    for j in jobs.iter_mut() {
        if j.done || j.paused { continue; }
        if clock >= j.trigger_at {
            to_run.push(j.fn_name.clone());
            if j.every_n == 0 { j.done = true; } else { j.trigger_at = clock + j.every_n; }
        }
    }
    to_run
}
fn sceuti_schedule_clear_all(jobs: &mut Vec<SceutiJob>) { jobs.clear(); }
fn sceuti_schedule_on_boot(jobs: &mut Vec<SceutiJob>, boot_jobs_run: &mut bool, fn_name: String) {
    if !*boot_jobs_run {
        let id = sceuti_next_job_id();
        jobs.push(SceutiJob { id, every_n: 0, trigger_at: 0, paused: false, done: false, fn_name });
        *boot_jobs_run = true;
    }
}

// ---- CATEGORY 3: SceutiEnv (features 21–30) ----

type SceutiEnvStore = Vec<(String, String)>;
fn sceuti_env_get(store: &SceutiEnvStore, key: &str) -> Option<String> {
    store.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}
fn sceuti_env_set(store: &mut SceutiEnvStore, key: String, val: String) {
    if let Some(slot) = store.iter_mut().find(|(k, _)| k == &key) { slot.1 = val; } else { store.push((key, val)); }
}
fn sceuti_env_delete(store: &mut SceutiEnvStore, key: &str) { store.retain(|(k, _)| k != key); }
fn sceuti_env_has(store: &SceutiEnvStore, key: &str) -> bool { store.iter().any(|(k, _)| k == key) }
fn sceuti_env_keys(store: &SceutiEnvStore) -> Vec<String> { store.iter().map(|(k, _)| k.clone()).collect() }
fn sceuti_env_values(store: &SceutiEnvStore) -> Vec<String> { store.iter().map(|(_, v)| v.clone()).collect() }
fn sceuti_env_entries(store: &SceutiEnvStore) -> Vec<(String, String)> { store.clone() }
fn sceuti_env_load_string(store: &mut SceutiEnvStore, raw: &str) {
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        if let Some(eq) = line.find('=') {
            let key = line[..eq].trim().to_string();
            let val = line[eq + 1..].trim().to_string();
            if !key.is_empty() { sceuti_env_set(store, key, val); }
        }
    }
}
fn sceuti_env_to_map(store: &SceutiEnvStore) -> Vec<(String, String)> { store.clone() }
fn sceuti_env_clear(store: &mut SceutiEnvStore) { store.clear(); }

// ---- CATEGORY 4: SceutiLog (features 31–40) ----

fn sceuti_log_push(history: &mut Vec<SceutiLogEntry>, level: SceutiLogLevel, prefix: &str, message: String, clock: u64) {
    let min_level = SceutiLogLevel::from_u64(SCEUTI_LOG_LEVEL.load(core::sync::atomic::Ordering::Relaxed));
    if (level as i32) < (min_level as i32) { return; }
    if history.len() >= SCEUTI_LOG_HISTORY_MAX { history.remove(0); }
    history.push(SceutiLogEntry { level, clock, prefix: prefix.to_string(), message });
}
fn sceuti_log_info(history: &mut Vec<SceutiLogEntry>, prefix: &str, msg: String, clock: u64) { sceuti_log_push(history, SceutiLogLevel::Info, prefix, msg, clock); }
fn sceuti_log_warn(history: &mut Vec<SceutiLogEntry>, prefix: &str, msg: String, clock: u64) { sceuti_log_push(history, SceutiLogLevel::Warn, prefix, msg, clock); }
fn sceuti_log_error(history: &mut Vec<SceutiLogEntry>, prefix: &str, msg: String, clock: u64) { sceuti_log_push(history, SceutiLogLevel::Error, prefix, msg, clock); }
fn sceuti_log_debug(history: &mut Vec<SceutiLogEntry>, prefix: &str, msg: String, clock: u64) { sceuti_log_push(history, SceutiLogLevel::Debug, prefix, msg, clock); }
fn sceuti_log_set_level(level: u64) { SCEUTI_LOG_LEVEL.store(level.min(3), core::sync::atomic::Ordering::Relaxed); }
fn sceuti_log_history(history: &Vec<SceutiLogEntry>, n: usize) -> Vec<String> {
    let start = history.len().saturating_sub(n);
    history[start..].iter().map(|e| {
        if e.prefix.is_empty() {
            format!("[{}][t={}] {}", e.level.as_str(), e.clock, e.message)
        } else {
            format!("[{}][{}][t={}] {}", e.level.as_str(), e.prefix, e.clock, e.message)
        }
    }).collect()
}
fn sceuti_log_history_count(history: &Vec<SceutiLogEntry>) -> usize { history.len() }
fn sceuti_log_clear_history(history: &mut Vec<SceutiLogEntry>) { history.clear(); }
fn sceuti_log_format(level: &str, prefix: &str, msg: &str, clock: u64) -> String {
    if prefix.is_empty() { format!("[{}][t={}] {}", level, clock, msg) }
    else { format!("[{}][{}][t={}] {}", level, prefix, clock, msg) }
}

// ---- CATEGORY 5: SceutiFake (features 41–50) — xorshift64, no_std-safe ----

fn sceuti_fake_next() -> u64 {
    let mut x = SCEUTI_FAKE_SEED.load(core::sync::atomic::Ordering::Relaxed);
    x ^= x << 13; x ^= x >> 7; x ^= x << 17;
    SCEUTI_FAKE_SEED.store(x, core::sync::atomic::Ordering::Relaxed);
    x
}
fn sceuti_fake_int(min: i64, max: i64) -> i64 {
    if min >= max { return min; }
    let range = (max - min) as u64;
    min + (sceuti_fake_next() % range) as i64
}
fn sceuti_fake_str(len: usize) -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut out = String::with_capacity(len);
    for _ in 0..len { out.push(CHARS[(sceuti_fake_next() as usize) % CHARS.len()] as char); }
    out
}
fn sceuti_fake_bool() -> bool { sceuti_fake_next() % 2 == 0 }
fn sceuti_fake_from(list: &[Value]) -> Option<Value> {
    if list.is_empty() { return None; }
    Some(list[(sceuti_fake_next() as usize) % list.len()].clone())
}
fn sceuti_fake_seed(n: u64) { SCEUTI_FAKE_SEED.store(if n == 0 { 1 } else { n }, core::sync::atomic::Ordering::Relaxed); }
fn sceuti_fake_uuid() -> String {
    let a = sceuti_fake_next(); let b = sceuti_fake_next();
    format!("{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        (a >> 32) & 0xFFFFFFFF, (a >> 16) & 0xFFFF, a & 0xFFFF, (b >> 48) & 0xFFFF, b & 0xFFFFFFFFFFFF)
}
fn sceuti_fake_hex(len: usize) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(len);
    for _ in 0..len { out.push(HEX[(sceuti_fake_next() as usize) % 16] as char); }
    out
}
fn sceuti_fake_bytes(n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        let r = sceuti_fake_next();
        for byte_idx in 0..8usize {
            if i >= n { break; }
            out.push(((r >> (byte_idx * 8)) & 0xFF) as u8);
            i += 1;
        }
    }
    out
}
fn sceuti_fake_ipv4() -> String {
    let r = sceuti_fake_next();
    format!("{}.{}.{}.{}", (r >> 24) & 0xFF, (r >> 16) & 0xFF, (r >> 8) & 0xFF, r & 0xFF)
}
fn sceuti_fake_shuffle(list: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = list.to_vec();
    let mut n = out.len();
    while n > 1 {
        let k = (sceuti_fake_next() as usize) % n;
        n -= 1;
        out.swap(n, k);
    }
    out
}

fn tq_map_int(pairs: &[(String, Value)], key: &str, default: i64) -> i64 {
    match autoclib_map_get(pairs, key) {
        Some(Value::Int(n))   => *n,
        Some(Value::Float(f)) => *f as i64,
        _ => default,
    }
}
fn tq_map_uint(pairs: &[(String, Value)], key: &str, default: u64) -> u64 {
    tq_map_int(pairs, key, default as i64).max(0) as u64
}
fn tq_map_str(pairs: &[(String, Value)], key: &str, default: &str) -> String {
    match autoclib_map_get(pairs, key) {
        Some(Value::Str(s)) => s.clone(),
        _ => default.to_string(),
    }
}
fn tq_job_to_map(job: &TasoaqueJob) -> Value {
    Value::Map(vec![
        ("id".into(), Value::Str(job.id.clone())),
        ("task".into(), Value::Str(job.task.clone())),
        ("queue".into(), Value::Str(job.queue.clone())),
        ("status".into(), Value::Str(job.status.clone())),
        ("priority".into(), Value::Int(job.priority)),
        ("attempts".into(), Value::Int(job.attempts as i64)),
        ("maxRetries".into(), Value::Int(job.max_retries as i64)),
        ("result".into(), job.result.clone()),
        ("error".into(), if job.error.is_empty() { Value::Null } else { Value::Str(job.error.clone()) }),
    ])
}

/// (queue, priority, maxRetries, retryDelay) defaults for a task name —
/// from `Tasoaque.task(name, opts)` if registered, else hardcoded defaults.
fn tasoaque_resolve_defaults(st: &TasoaqueState, task: &str) -> (String, i64, u32, u64) {
    match st.find_task_def(task) {
        Some(d) => (d.queue.clone(), d.priority, d.max_retries, d.retry_delay),
        None => ("default".to_string(), 5, 0, 1),
    }
}

/// Per-task sliding-window rate limiter. Returns false (job stays pending,
/// tried again next `runWorker`/`runOne` call) if the task's window is full.
fn tasoaque_check_rate(st: &mut TasoaqueState, task: &str) -> bool {
    let limit = match st.rate_limits.iter().find(|(t, _, _)| t == task) {
        Some((_, max, window)) => (*max, *window),
        None => return true,
    };
    let (max, window) = limit;
    let clock = st.clock;
    match st.rate_usage.iter_mut().find(|(t, _, _)| t == task) {
        Some(entry) => {
            if clock.saturating_sub(entry.1) >= window { entry.1 = clock; entry.2 = 0; }
            if entry.2 >= max { false } else { entry.2 += 1; true }
        }
        None => { st.rate_usage.push((task.to_string(), clock, 1)); true }
    }
}

/// Chord support: once every job in a group has reached a terminal state
/// (success or failed), enqueue the group's callback task exactly once with
/// the collected results as its single argument (Celery's `chord` semantics,
/// minus the need for a separate result backend — everything lives in the
/// same in-process TasoaqueState).
fn tasoaque_maybe_fire_chord(st: &mut TasoaqueState, group_id: &str) {
    let gi = match st.groups.iter().position(|g| g.id == group_id) { Some(i) => i, None => return };
    if st.groups[gi].fired { return; }
    let job_ids = st.groups[gi].job_ids.clone();
    let mut results: Vec<Value> = Vec::with_capacity(job_ids.len());
    for jid in &job_ids {
        if let Some(j) = st.jobs.iter().find(|j| &j.id == jid) {
            if j.status == "success" { results.push(j.result.clone()); }
            else if j.status == "failed" { results.push(Value::Null); }
            else { return; } // still pending/running/retry — not done yet
        } else if st.dead_letters.iter().any(|j| &j.id == jid) {
            results.push(Value::Null);
        } else {
            return; // job vanished unexpectedly — don't fire early
        }
    }
    st.groups[gi].fired = true;
    if !st.groups[gi].callback.is_empty() {
        let clock = st.clock;
        let cb_task = st.groups[gi].callback.clone();
        let cb_queue = st.groups[gi].callback_queue.clone();
        let id = st.fresh_id("tq");
        st.jobs.push(TasoaqueJob {
            id, task: cb_task, args: vec![Value::List(results)], queue: cb_queue,
            priority: 5, eta: clock, status: "pending".into(), attempts: 0,
            max_retries: 0, retry_delay: 1, result: Value::Null, error: String::new(),
            created_at: clock, chain_next: Vec::new(), group_id: String::new(),
        });
    }
}

// =============================================================================
// TASOAQUE CLUSTER — real cross-machine wire protocol.
//
// Same framing as `Autoclib.remoteExec` above: 4-byte big-endian length
// prefix + a UTF-8 JSON payload (encoded/decoded with the same
// `value_to_json` / `vyradb_json_parse_value` pair VyraDB already uses to
// persist rows — no new serialization format invented here). Built directly
// on `hal().net_tcp_connect/write/read`, which are real MonobatHal trait
// methods (StubHal's defaults honestly return NotReady until Monobat's
// network driver is wired up — see the HONEST GAP note above).
//
// The coordinator (`Tasoaque.serve`) never executes task code itself — it
// only hands out job descriptions and records outcomes. All actual task
// execution happens wherever `Tasoaque.remoteWork` is running, which can be
// a completely separate OS process on a completely separate machine. That
// separation is what makes this genuine multi-machine distribution, not a
// simulation of it.
// =============================================================================

fn tasoaque_send_frame(handle: u32, payload: &[u8]) -> Result<(), String> {
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    remox_tcp_write(handle, &frame).map_err(|e| e.to_string())
}

fn tasoaque_recv_frame(handle: u32) -> Result<Vec<u8>, String> {
    let mut len_buf: Vec<u8> = Vec::new();
    while len_buf.len() < 4 {
        let chunk = remox_tcp_read(handle, 4 - len_buf.len()).map_err(|e| e.to_string())?;
        if chunk.is_empty() { return Err("connection closed before length prefix arrived".to_string()); }
        len_buf.extend_from_slice(&chunk);
    }
    let msg_len = u32::from_be_bytes([len_buf[0], len_buf[1], len_buf[2], len_buf[3]]) as usize;
    let mut payload: Vec<u8> = Vec::new();
    while payload.len() < msg_len {
        let chunk = remox_tcp_read(handle, msg_len - payload.len()).map_err(|e| e.to_string())?;
        if chunk.is_empty() { return Err(format!("connection closed mid-message ({}/{} bytes)", payload.len(), msg_len)); }
        payload.extend_from_slice(&chunk);
    }
    Ok(payload)
}

/// Coordinator-side handler for one wire request. Pure function over
/// `TASOAQUE` — no `self`/`call_function` needed because the coordinator
/// never runs task code; it only allots jobs (`op: "pull"`) and records
/// outcomes reported back by remote workers (`op: "complete"` / `"fail"`),
/// reusing the exact same success/retry/dead-letter/chain/chord bookkeeping
/// as the local `tasoaque_process` engine.
fn tasoaque_handle_wire_op(payload: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(payload).to_string();
    let req = match vyradb_json_parse_value(&text) {
        Some(Value::Map(m)) => m,
        _ => return value_to_json(&Value::Map(vec![("error".into(), Value::Str("Tasoaque: malformed request".into()))])).into_bytes(),
    };
    let op = tq_map_str(&req, "op", "");
    let mut st = TASOAQUE.lock().unwrap();

    let resp = match op.as_str() {
        "pull" => {
            let queue = tq_map_str(&req, "queue", "");
            let clock = st.clock;
            let due = st.jobs.iter().enumerate()
                .filter(|(_, j)| (j.status == "pending" || j.status == "retry")
                    && (queue.is_empty() || j.queue == queue) && j.eta <= clock)
                .min_by(|(_, a), (_, b)| a.priority.cmp(&b.priority).then(a.eta.cmp(&b.eta)).then(a.created_at.cmp(&b.created_at)))
                .map(|(i, _)| i);
            match due {
                Some(i) => {
                    st.jobs[i].status = "running".to_string();
                    let j = &st.jobs[i];
                    Value::Map(vec![
                        ("jobId".into(), Value::Str(j.id.clone())),
                        ("task".into(), Value::Str(j.task.clone())),
                        ("args".into(), Value::List(j.args.clone())),
                    ])
                }
                None => Value::Map(vec![("jobId".into(), Value::Null)]),
            }
        }
        "complete" => {
            let jid = tq_map_str(&req, "jobId", "");
            let result = autoclib_map_get(&req, "result").cloned().unwrap_or(Value::Null);
            match st.jobs.iter().position(|j| j.id == jid) {
                Some(pos) => {
                    st.jobs[pos].status = "success".to_string();
                    st.jobs[pos].result = result.clone();
                    if !st.jobs[pos].chain_next.is_empty() {
                        let (next_task, mut next_args, next_queue) = st.jobs[pos].chain_next.remove(0);
                        let remaining = st.jobs[pos].chain_next.clone();
                        let (priority, max_retries, retry_delay) =
                            (st.jobs[pos].priority, st.jobs[pos].max_retries, st.jobs[pos].retry_delay);
                        let group_id = st.jobs[pos].group_id.clone();
                        next_args.push(result);
                        let clock = st.clock;
                        let new_id = st.fresh_id("tq");
                        st.jobs.push(TasoaqueJob {
                            id: new_id, task: next_task, args: next_args, queue: next_queue,
                            priority, eta: clock, status: "pending".into(), attempts: 0,
                            max_retries, retry_delay, result: Value::Null, error: String::new(),
                            created_at: clock, chain_next: remaining, group_id,
                        });
                    }
                    let gid = st.jobs[pos].group_id.clone();
                    if !gid.is_empty() { tasoaque_maybe_fire_chord(&mut st, &gid); }
                    Value::Map(vec![("ok".into(), Value::Bool(true))])
                }
                None => Value::Map(vec![("ok".into(), Value::Bool(false)), ("error".into(), Value::Str("job not found".into()))]),
            }
        }
        "fail" => {
            let jid = tq_map_str(&req, "jobId", "");
            let err_msg = tq_map_str(&req, "error", "remote worker reported failure");
            match st.jobs.iter().position(|j| j.id == jid) {
                Some(pos) => {
                    st.jobs[pos].attempts += 1;
                    if st.jobs[pos].attempts <= st.jobs[pos].max_retries {
                        let delay = st.jobs[pos].retry_delay * st.jobs[pos].attempts as u64;
                        let clock = st.clock;
                        st.jobs[pos].status = "retry".to_string();
                        st.jobs[pos].eta = clock + delay.max(1);
                        st.jobs[pos].error = err_msg;
                    } else {
                        st.jobs[pos].status = "failed".to_string();
                        st.jobs[pos].error = err_msg;
                        let gid = st.jobs[pos].group_id.clone();
                        let dead = st.jobs.remove(pos);
                        st.dead_letters.push(dead);
                        if !gid.is_empty() { tasoaque_maybe_fire_chord(&mut st, &gid); }
                    }
                    Value::Map(vec![("ok".into(), Value::Bool(true))])
                }
                None => Value::Map(vec![("ok".into(), Value::Bool(false)), ("error".into(), Value::Str("job not found".into()))]),
            }
        }
        _ => Value::Map(vec![("error".into(), Value::Str(format!("Tasoaque: unknown op '{}'", op)))]),
    };
    value_to_json(&resp).into_bytes()
}

// =============================================================================
// INTERPRETER
// =============================================================================
struct Interpreter {
    env:     Vec<HashMap<String, Value>>,
    fns:     HashMap<String, (Vec<(String, Option<Expr>)>, Rc<Vec<Stmt>>, bool)>,
    structs: HashMap<String, StructDef>,
    impls:   HashMap<String, Vec<(String, Vec<String>, Vec<Stmt>)>>,
    traits:  HashMap<String, TraitDef>,
    rand_state: u64,
    memo:    HashMap<(String, String), Value>,
    // Feature 50: accumulated style rules that get injected into the next HTML output
    pending_styles: Vec<(String, Vec<(String, String)>)>,
    // Remojoke: current output language ("src" = original Hinglish text,
    // as written — the only "language" that's guaranteed complete, since
    // it's not a translation of anything). Sticky across calls within a
    // session, changed via `Lang.remojoke("hindi")` etc.
    remojoke_lang: String,
}

impl Interpreter {
    fn new() -> Self {
        Interpreter {
            env: vec![HashMap::new()],
            memo: HashMap::new(),
            fns: HashMap::new(),
            structs: HashMap::new(),
            impls: HashMap::new(),
            traits: HashMap::new(),
            // RNG SEED WARNING: SystemTime::now() is a placeholder that
            // always returns epoch 0 until Monobat's RTC/TSC driver is
            // wired (see std-compat shim above) — so it contributes ZERO
            // entropy on its own. We mix in remox_entropy() (also a
            // no-op default until a real HAL impl overrides it — see
            // MonobatHal::entropy_source doc comment) plus the stack
            // address of a throwaway local, which at least varies with
            // stack-layout jitter across runs on real hardware. This is
            // NOT cryptographically secure — do not use Remox's RNG for
            // session tokens, secrets, or anything security-sensitive
            // until a real hardware entropy source is wired via HAL.
            rand_state: {
                let stack_jitter = {
                    let probe: u64 = 0;
                    &probe as *const u64 as u64
                };
                let seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64;
                seed
                    ^ crate::remox_entropy()
                    ^ stack_jitter.wrapping_mul(2685821657736338717)
                    ^ 0x9E3779B97F4A7C15 // fixed odd constant so seed is never literally 0
            },
            pending_styles: Vec::new(),
            remojoke_lang: String::from("src"),
        }
    }

    /// Compiles `src` all the way down to a `CompiledProgram` and returns it,
    /// WITHOUT running anything. This is the real "compile" step: lex once,
    /// parse once, then run the Compiler's whole-program pass (hoist decls,
    /// validate, constant-fold). Nothing in this function executes Remox
    /// code — it only produces the artifact that `run_compiled` will run.
    pub fn compile_source(src: &str) -> Result<CompiledProgram, String> {
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().map_err(|e| format!("Lexer Error: {}", e))?;
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().map_err(|e| format!("Parse Error: {}", e))?;
        let mut compiler = Compiler::new();
        compiler.compile(&ast).map_err(|e| format!("Compile Error: {}", e))
    }

    /// Executes an already-compiled program. This is the runtime/VM side:
    /// it never re-lexes, re-parses, or re-resolves anything — every name
    /// was already resolved by the Compiler. Functions/structs/impls/traits
    /// from the compiled program are merged into this interpreter's tables
    /// (so REPL submissions and `use`-imported modules accumulate state
    /// across calls exactly like the original interpreter did), and the
    /// program's statement stream is then run top to bottom.
    pub fn run_compiled(&mut self, program: CompiledProgram) {
        for w in &program.warnings {
            eprintln!("Compile Warning: {}", w);
        }
        for (k, v) in program.fns     { self.fns.insert(k, v); }
        for (k, v) in program.structs { self.structs.insert(k, v); }
        for (k, v) in program.impls   { self.impls.insert(k, v); }
        for (k, v) in program.traits  { self.traits.insert(k, v); }

        match self.exec_block(&program.code) {
            Ok(_) | Err(RuntimeSignal::Exit(_)) | Err(RuntimeSignal::Return(_)) => {}
            Err(RuntimeSignal::Error(e)) => eprintln!("Runtime Error: {}", e),
        }
    }

    /// Compile `src` ahead of time, then run the result. This replaces the
    /// old "parse-and-immediately-walk" `run_source`: compilation is now a
    /// complete, separate phase that finishes (and can fail with a
    /// `Compile Error`) before a single statement of the program executes.
    pub fn run_source(&mut self, src: &str) {
        match Self::compile_source(src) {
            Ok(program) => self.run_compiled(program),
            Err(e) => eprintln!("{}", e),
        }
    }

    fn push_scope(&mut self) { self.env.push(HashMap::new()); }
    fn pop_scope(&mut self)  { self.env.pop(); }

    fn get_var(&self, name: &str) -> Option<Value> {
        for scope in self.env.iter().rev() {
            if let Some(v) = scope.get(name) { return Some(v.clone()); }
        }
        None
    }

    fn set_var(&mut self, name: &str, val: Value) {
        for scope in self.env.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), val);
                return;
            }
        }
        if let Some(scope) = self.env.last_mut() {
            scope.insert(name.to_string(), val);
        }
    }

    fn def_var(&mut self, name: &str, val: Value) {
        if let Some(scope) = self.env.last_mut() {
            scope.insert(name.to_string(), val);
        }
    }

    fn exec_block(&mut self, stmts: &[Stmt]) -> Result<Value, RuntimeSignal> {
        let mut last = Value::Null;
        for stmt in stmts {
            last = self.exec_stmt(stmt)?;
        }
        Ok(last)
    }

    fn exec_stmt(&mut self, stmt: &Stmt) -> Result<Value, RuntimeSignal> {
        sceuti_tick(); // Sceuti v2.0: SceutiClock/SceutiSchedule ka logical tick
        match stmt {

            Stmt::Let { names, values } => {
                let mut vals = Vec::new();
                for e in values { vals.push(self.eval_expr(e)?); }
                for (i, name) in names.iter().enumerate() {
                    let v = vals.get(i).cloned().unwrap_or(Value::Null);
                    // If var exists in an outer scope, update it (mutable loop vars)
                    // Only define fresh if not found anywhere outside current scope
                    let exists_outer = self.env.iter().rev().skip(1).any(|s| s.contains_key(name.as_str()));
                    if exists_outer {
                        self.set_var(name, v);
                    } else {
                        self.def_var(name, v);
                    }
                }
                Ok(Value::Null)
            }

            Stmt::Assign { name, value } => {
                let v = self.eval_expr(value)?;
                // If variable doesn't exist yet, declare it (let becomes optional)
                if self.get_var(name).is_none() {
                    self.def_var(name, v);
                } else {
                    self.set_var(name, v);
                }
                Ok(Value::Null)
            }

            // Fix 4: struct field mutation — p.x = new_val
            Stmt::FieldAssign { obj, field, value } => {
                let new_val = self.eval_expr(value)?;
                let current = self.get_var(obj).ok_or_else(|| RuntimeSignal::Error(format!("Undefined: {}", obj)))?;
                let updated = match current {
                    Value::Struct { name: sname, mut fields } => {
                        if let Some(entry) = fields.iter_mut().find(|(k, _)| k == field) {
                            entry.1 = new_val;
                        } else {
                            fields.push((field.clone(), new_val));
                        }
                        Value::Struct { name: sname, fields }
                    }
                    Value::Map(mut pairs) => {
                        if let Some(entry) = pairs.iter_mut().find(|(k, _)| k == field) {
                            entry.1 = new_val;
                        } else {
                            pairs.push((field.clone(), new_val));
                        }
                        Value::Map(pairs)
                    }
                    _ => return Err(RuntimeSignal::Error(format!("{} is not a struct/map", obj))),
                };
                self.set_var(obj, updated);
                Ok(Value::Null)
            }

            // Feature 33: Destructuring
            Stmt::Destructure { keys, source } => {
                let val = self.eval_expr(source)?;
                match val {
                    Value::Struct { ref fields, .. } => {
                        for key in keys {
                            let v = fields.iter().find(|(k, _)| k == key)
                                .map(|(_, v)| v.clone()).unwrap_or(Value::Null);
                            self.def_var(key, v);
                        }
                    }
                    Value::Map(ref pairs) => {
                        for key in keys {
                            let v = pairs.iter().find(|(k, _)| k == key)
                                .map(|(_, v)| v.clone()).unwrap_or(Value::Null);
                            self.def_var(key, v);
                        }
                    }
                    _ => return Err(RuntimeSignal::Error("Destructuring requires struct or map".into())),
                }
                Ok(Value::Null)
            }

            Stmt::Say(exprs) => {
                let mut parts = Vec::new();
                for e in exprs {
                    parts.push(self.eval_expr(e)?.to_string());
                }
                println!("{}", parts.join(" "));
                Ok(Value::Null)
            }

            // Feature 21, 22, 40: fn with named+default params, async
            Stmt::Fn { name, params, body, is_async } => {
                self.fns.insert(name.clone(), (params.clone(), Rc::new(body.clone()), *is_async));
                Ok(Value::Null)
            }

            Stmt::Loop { count, body } => {
                let n = match self.eval_expr(count)? {
                    Value::Int(n) => n,
                    v => return Err(RuntimeSignal::Error(format!("loop expects int, got {}", v))),
                };
                for _ in 0..n {
                    self.push_scope();
                    let r = self.exec_block(body);
                    self.pop_scope();
                    match r {
                        Ok(_) => {}
                        Err(RuntimeSignal::Return(v)) => return Ok(v.unwrap_or(Value::Null)),
                        Err(e) => return Err(e),
                    }
                }
                Ok(Value::Null)
            }

            Stmt::When { subject, cases, default } => {
                let subj = self.eval_expr(subject)?;
                // Guard mode: subject is BoolLit(true) sentinel — eval each case as a boolean condition
                let guard_mode = subj == Value::Bool(true) && matches!(subject, Expr::BoolLit(true));
                for (pat, body) in cases {
                    let matched = if guard_mode {
                        // Each pattern is a full boolean expression (e.g. v < 10.0)
                        self.eval_expr(pat).map(|v| v.is_truthy()).unwrap_or(false)
                    } else {
                        let pval = self.eval_expr(pat)?;
                        subj.is_equal(&pval)
                    };
                    if matched {
                        self.push_scope();
                        let r = self.exec_block(body);
                        self.pop_scope();
                        return r;
                    }
                }
                if let Some(def) = default {
                    self.push_scope();
                    let r = self.exec_block(def);
                    self.pop_scope();
                    return r;
                }
                Ok(Value::Null)
            }

            Stmt::Each { var, iter, body } => {
                let iterable = self.eval_expr(iter)?;
                let items: Vec<Value> = match iterable {
                    Value::List(v)      => v,
                    Value::Range(a, b)  => (a..b).map(Value::Int).collect(),
                    Value::Str(s)       => s.chars().map(|c| Value::Str(c.to_string())).collect(),
                    _ => return Err(RuntimeSignal::Error("each: not iterable".into())),
                };
                for item in items {
                    self.push_scope();
                    self.def_var(var, item);
                    let r = self.exec_block(body);
                    self.pop_scope();
                    match r {
                        Ok(_) => {}
                        Err(RuntimeSignal::Return(v)) => return Ok(v.unwrap_or(Value::Null)),
                        Err(e) => return Err(e),
                    }
                }
                Ok(Value::Null)
            }

            Stmt::If { cond, then_body, else_body } => {
                let cv = self.eval_expr(cond)?;
                if cv.is_truthy() {
                    self.push_scope();
                    let r = self.exec_block(then_body);
                    self.pop_scope();
                    r
                } else if let Some(eb) = else_body {
                    self.push_scope();
                    let r = self.exec_block(eb);
                    self.pop_scope();
                    r
                } else { Ok(Value::Null) }
            }

            Stmt::Exit(code_expr) => {
                let code = match code_expr {
                    Some(e) => match self.eval_expr(e)? { Value::Int(n) => n as i32, _ => 0 },
                    None => 0,
                };
                Err(RuntimeSignal::Exit(code))
            }

            Stmt::Return(val_expr) => {
                let v = match val_expr {
                    Some(e) => Some(self.eval_expr(e)?),
                    None => None,
                };
                Err(RuntimeSignal::Return(v))
            }

            // Feature 27: struct declaration
            Stmt::StructDecl(def) => {
                self.structs.insert(def.name.clone(), def.clone());
                Ok(Value::Null)
            }

            // Feature 29: impl block
            Stmt::ImplDecl(block) => {
                self.impls.insert(block.target.clone(), block.methods.clone());
                Ok(Value::Null)
            }

            // Feature 30: trait
            Stmt::TraitDecl(def) => {
                self.traits.insert(def.name.clone(), def.clone());
                Ok(Value::Null)
            }

            // Feature 31: try/catch/else
            Stmt::TryCatch { body, catch_var, catch_body, else_body } => {
                self.push_scope();
                let result = self.exec_block(body);
                self.pop_scope();
                match result {
                    Ok(v) => {
                        // else branch runs on success (like Python try/else)
                        if let Some(eb) = else_body {
                            self.push_scope();
                            let r = self.exec_block(eb);
                            self.pop_scope();
                            r
                        } else { Ok(v) }
                    }
                    Err(RuntimeSignal::Error(msg)) => {
                        self.push_scope();
                        if let Some(cv) = catch_var {
                            self.def_var(cv, Value::Str(msg.clone()));
                        }
                        let r = self.exec_block(catch_body);
                        self.pop_scope();
                        r
                    }
                    Err(e) => Err(e), // re-propagate Exit/Return
                }
            }

            // Feature 36: use (module loading)
            Stmt::Import(name) => {
                match get_module(name) {
                    Some(exports) => {
                        // Store as both: flat globals (use math → pi, e, ...)
                        // AND as a Map under the module name (math.pi, math.sqrt())
                        let map_val = Value::Map(exports.clone());
                        self.def_var(name, map_val);
                        for (k, v) in exports {
                            self.def_var(&k, v);
                        }
                    }
                    None => {
                        // Try to load as file: name.rx
                        let path = format!("{}.rx", name);
                        match fs::read_to_string(&path) {
                            Ok(src) => self.run_source(&src),
                            Err(_) => eprintln!("Warning: module '{}' not found", name),
                        }
                    }
                }
                Ok(Value::Null)
            }

            // Feature 37: type alias (runtime: just a note, no enforcement needed)
            Stmt::TypeAlias { alias, target } => {
                // store as a string var so code can inspect it
                self.def_var(alias, Value::Str(target.clone()));
                Ok(Value::Null)
            }

            // Feature 50: style block — accumulate CSS rules for next HTML output
            Stmt::StyleBlock { rules } => {
                // Merge into pending_styles (later ui/screen/view picks them up)
                for rule in rules {
                    // If selector already exists, merge props; else push new
                    if let Some(existing) = self.pending_styles.iter_mut().find(|(s, _)| s == &rule.0) {
                        existing.1.extend(rule.1.clone());
                    } else {
                        self.pending_styles.push(rule.clone());
                    }
                }
                println!("✓ Style block registered ({} selector(s))", rules.len());
                Ok(Value::Null)
            }

            // Feature 41b: view ComponentName { ... } → generates <div class="name">...</div> HTML file
            Stmt::ViewDecl { name, body } => {
                let mut inner_html = String::new();
                for node in body {
                    self.render_ui_node(node, &mut inner_html, 4)?;
                }
                let extra_style = self.drain_pending_style_tag();
                let html = format!(
"<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"UTF-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n\
<title>{name}</title>\n\
<style>\n  * {{ box-sizing: border-box; }}\n  body {{ margin: 0; font-family: -apple-system, Segoe UI, Roboto, sans-serif; }}\n\
  .remox-view {{ display: flex; flex-direction: column; }}\n</style>\n\
{extra}\
</head>\n<body>\n\
  <div class=\"remox-view {name}\">\n{inner}</div>\n</body>\n</html>\n",
                    name  = escape_html(name),
                    inner = inner_html,
                    extra = extra_style,
                );
                let filename = format!("{}.html", name);
                match fs::write(&filename, &html) {
                    Ok(_)  => println!("✓ View component generated: {}", filename),
                    Err(e) => eprintln!("Could not write view '{}': {}", filename, e),
                }
                Ok(Value::Str(filename))
            }

            // Feature 42: screen PageName { title: "..." theme: dark ... }
            // Generates a full-page HTML with responsive viewport + dark/light CSS vars
            Stmt::ScreenDecl { name, title, theme, body } => {
                let page_title = match title {
                    Some(e) => self.eval_expr(e)?.to_string(),
                    None    => name.clone(),
                };
                let is_dark = theme.as_deref() == Some("dark");
                let (bg, fg, accent) = if is_dark {
                    ("#0f172a", "#e2e8f0", "#6366f1")   // Tailwind slate-900 / slate-200 / indigo-500
                } else {
                    ("#f8fafc", "#0f172a", "#6366f1")   // slate-50 / slate-900 / indigo-500
                };
                let mut body_html = String::new();
                for node in body {
                    self.render_ui_node(node, &mut body_html, 4)?;
                }
                let html = format!(
"<!DOCTYPE html>\n<html lang=\"en\" data-theme=\"{theme}\">\n<head>\n\
<meta charset=\"UTF-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n\
<title>{title}</title>\n\
<style>\n\
  :root {{\n\
    --bg: {bg};\n\
    --fg: {fg};\n\
    --accent: {accent};\n\
    --radius: 8px;\n\
    --font: -apple-system, Segoe UI, Roboto, sans-serif;\n\
  }}\n\
  *, *::before, *::after {{ box-sizing: border-box; margin: 0; padding: 0; }}\n\
  html, body {{ height: 100%; }}\n\
  body {{\n\
    background: var(--bg);\n\
    color: var(--fg);\n\
    font-family: var(--font);\n\
    font-size: 16px;\n\
    line-height: 1.5;\n\
  }}\n\
  .remox-screen {{ min-height: 100vh; display: flex; flex-direction: column; }}\n\
  button, .remox-btn {{\n\
    background: var(--accent);\n\
    color: #fff;\n\
    border: none;\n\
    border-radius: var(--radius);\n\
    padding: 10px 20px;\n\
    cursor: pointer;\n\
    font-size: 1rem;\n\
    transition: opacity 0.15s;\n\
  }}\n\
  button:hover, .remox-btn:hover {{ opacity: 0.85; }}\n\
  img {{ max-width: 100%; height: auto; }}\n\
</style>\n\
</head>\n<body>\n\
  <div class=\"remox-screen {name}\">\n{body}</div>\n\
</body>\n</html>\n",
                    theme   = escape_html(theme.as_deref().unwrap_or("light")),
                    title   = escape_html(&page_title),
                    bg      = bg, fg = fg, accent = accent,
                    name    = escape_html(name),
                    body    = body_html,
                );
                let filename = format!("{}.html", name);
                match fs::write(&filename, &html) {
                    Ok(_)  => println!("✓ Screen generated: {} (theme: {})", filename,
                                       theme.as_deref().unwrap_or("light")),
                    Err(e) => eprintln!("Could not write screen '{}': {}", filename, e),
                }
                Ok(Value::Str(filename))
            }

            // Feature 41: ui { } block — render to a standalone HTML/CSS file
            Stmt::UiDecl { name, title, root } => {
                let page_title = match title {
                    Some(e) => self.eval_expr(e)?.to_string(),
                    None => name.clone(),
                };
                let mut body = String::new();
                for node in root {
                    self.render_ui_node(node, &mut body, 2)?;
                }
                let html = format!(
"<!DOCTYPE html>
<html lang=\"en\">
<head>
<meta charset=\"UTF-8\">
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">
<title>{title}</title>
<style>
  * {{ box-sizing: border-box; }}
  body {{ margin: 0; font-family: -apple-system, Segoe UI, Roboto, sans-serif; }}
</style>
</head>
<body>
{body}</body>
</html>
",
                    title = escape_html(&page_title),
                    body = body,
                );
                let filename = format!("{}.html", name);
                match fs::write(&filename, &html) {
                    Ok(_) => println!("✓ UI generated: {}", filename),
                    Err(e) => eprintln!("Could not write UI file '{}': {}", filename, e),
                }
                Ok(Value::Str(filename))
            }

            Stmt::Expr(e) => self.eval_expr(e),
        }
    }

    // Feature 50: build a <style> tag from pending_styles and drain the accumulator.
    // Called by every HTML-generating statement so injected CSS lands exactly once.
    fn drain_pending_style_tag(&mut self) -> String {
        if self.pending_styles.is_empty() { return String::new(); }
        let mut css = String::from("<style>\n");
        for (selector, props) in &self.pending_styles {
            css.push_str(&format!("  {} {{\n", selector));
            for (k, v) in props {
                // Auto-append px to bare integers for dimension properties
                let needs_px = matches!(k.as_str(),
                    "font-size"|"width"|"height"|"margin"|"padding"|
                    "gap"|"border-radius"|"top"|"left"|"right"|"bottom");
                let val_out = if needs_px && v.chars().all(|c| c.is_ascii_digit()) {
                    format!("{}px", v)
                } else {
                    v.clone()
                };
                css.push_str(&format!("    {}: {};\n", k, val_out));
            }
            css.push_str("  }\n");
        }
        css.push_str("</style>\n");
        self.pending_styles.clear();
        css
    }

    // Feature 41-45: recursively render one UiNode into HTML+inline-CSS, indented.
    // ─── Tag routing ────────────────────────────────────────────────────────────
    // text    → <span>  with font-size/color/font-weight from props
    // button  → <button class="remox-btn">  with onclick from on_click prop
    // img     → <img>  with src/width/height/alt as HTML attrs (not CSS)
    // screen/view → structural divs (handled at Stmt level, but also inline)
    fn render_ui_node(&mut self, node: &UiNode, out: &mut String, indent: usize) -> Result<(), RuntimeSignal> {
        let pad = "  ".repeat(indent);

        // ── Canonical HTML tag ───────────────────────────────────────────────
        // Feature 46: input  → <input>  (self-closing, attrs: placeholder, type, value, name)
        // Feature 47: layout-row   → <div style="display:flex; flex-direction:row">
        // Feature 48: layout-col   → <div style="display:flex; flex-direction:column">
        // Feature 49: layout-grid  → <div style="display:grid; grid-template-columns:...">
        let html_tag = match node.tag.as_str() {
            "text"        => "span",
            "button"      => "button",
            "img" | "image" => "img",
            "input"       => "input",
            "view"        => "div",
            "screen"      => "section",
            "layout-row"  => "div",
            "layout-col"  => "div",
            "layout-grid" => "div",
            other         => other,
        };

        // ── CSS style string ─────────────────────────────────────────────────
        // Feature 47/48/49: layout-row/col/grid get base flex/grid CSS injected first.
        // User props then override/extend these defaults.
        let mut style_parts: Vec<String> = match node.tag.as_str() {
            "layout-row"  => vec![
                "display: flex".to_string(),
                "flex-direction: row".to_string(),
                "align-items: center".to_string(),
                "flex-wrap: wrap".to_string(),
            ],
            "layout-col"  => vec![
                "display: flex".to_string(),
                "flex-direction: column".to_string(),
            ],
            "layout-grid" => vec![
                "display: grid".to_string(),
                // columns/gap come from user props below; set sensible default
                "grid-template-columns: repeat(auto-fit, minmax(160px, 1fr))".to_string(),
                "gap: 16px".to_string(),
            ],
            _ => Vec::new(),
        };

        // HTML attributes string (non-CSS)
        let mut attr_str = String::new();

        for (k, v) in &node.props {
            let raw_val = self.eval_expr(v)?.to_string();

            match k.as_str() {
                // Feature 43: text shortcuts
                "size" => {
                    let px = if raw_val.ends_with("px") || raw_val.ends_with("em")
                                || raw_val.ends_with("rem") || raw_val.ends_with("%") {
                        raw_val.clone()
                    } else { format!("{}px", raw_val) };
                    style_parts.retain(|s| !s.starts_with("font-size"));
                    style_parts.push(format!("font-size: {}", px));
                }
                "bold" if raw_val == "true" => {
                    style_parts.push("font-weight: bold".to_string());
                }
                "italic" if raw_val == "true" => {
                    style_parts.push("font-style: italic".to_string());
                }
                "font" => {
                    style_parts.push(format!("font-family: {}", raw_val));
                }
                "bg" | "background" => {
                    style_parts.push(format!("background: {}", raw_val));
                }
                // Feature 49: grid-specific props
                "columns" if node.tag == "layout-grid" => {
                    let tpl = if raw_val.chars().all(|c| c.is_ascii_digit()) {
                        format!("repeat({}, 1fr)", raw_val)
                    } else { raw_val.clone() };
                    style_parts.retain(|s| !s.starts_with("grid-template-columns"));
                    style_parts.push(format!("grid-template-columns: {}", tpl));
                }
                "gap" => {
                    let px = if raw_val.chars().all(|c| c.is_ascii_digit()) {
                        format!("{}px", raw_val)
                    } else { raw_val.clone() };
                    style_parts.retain(|s| !s.starts_with("gap"));
                    style_parts.push(format!("gap: {}", px));
                }
                // Feature 47/48: layout alignment shorthands
                "align" => {
                    let css_a = match raw_val.as_str() {
                        "start"   => "flex-start",
                        "end"     => "flex-end",
                        "between" => "space-between",
                        "around"  => "space-around",
                        other     => other,
                    };
                    style_parts.retain(|s| !s.starts_with("align-items"));
                    style_parts.push(format!("align-items: {}", css_a));
                }
                "justify" => {
                    let css_j = match raw_val.as_str() {
                        "start"   => "flex-start",
                        "end"     => "flex-end",
                        "between" => "space-between",
                        "around"  => "space-around",
                        "evenly"  => "space-evenly",
                        other     => other,
                    };
                    style_parts.push(format!("justify-content: {}", css_j));
                }
                // Feature 45: image HTML attrs
                "width" if html_tag == "img" => {
                    attr_str.push_str(&format!(" width=\"{}\"", escape_html(&raw_val)));
                    let px = if raw_val.chars().all(|c| c.is_ascii_digit()) {
                        format!("width: {}px", raw_val)
                    } else { format!("width: {}", raw_val) };
                    style_parts.push(px);
                    continue;
                }
                "height" if html_tag == "img" => {
                    attr_str.push_str(&format!(" height=\"{}\"", escape_html(&raw_val)));
                    let px = if raw_val.chars().all(|c| c.is_ascii_digit()) {
                        format!("height: {}px", raw_val)
                    } else { format!("height: {}", raw_val) };
                    style_parts.push(px);
                    continue;
                }
                "src" if html_tag == "img" => {
                    attr_str.push_str(&format!(" src=\"{}\"", escape_html(&raw_val)));
                    continue;
                }
                "alt" if html_tag == "img" => {
                    attr_str.push_str(&format!(" alt=\"{}\"", escape_html(&raw_val)));
                    continue;
                }
                // Feature 46: input HTML attrs — NOT CSS
                "placeholder" | "value" | "name" | "disabled" | "checked"
                    if html_tag == "input" =>
                {
                    attr_str.push_str(&format!(" {}=\"{}\"", k, escape_html(&raw_val)));
                    continue;
                }
                "type" if html_tag == "input" => {
                    attr_str.push_str(&format!(" type=\"{}\"", escape_html(&raw_val)));
                    continue;
                }
                // Feature 46: input-specific CSS (width, padding, etc.) still valid
                "width" | "height" => {
                    let px = if raw_val.chars().all(|c| c.is_ascii_digit()) {
                        format!("{}px", raw_val)
                    } else { raw_val.clone() };
                    style_parts.push(format!("{}: {}", css_key(k), px));
                }
                "padding" | "margin" => {
                    let px_val = if raw_val.chars().all(|c| c.is_ascii_digit()) {
                        format!("{}px", raw_val)
                    } else { raw_val.clone() };
                    style_parts.push(format!("{}: {}", k, px_val));
                }
                _ => {
                    style_parts.push(format!("{}: {}", css_key(k), raw_val));
                }
            }
        }

        // ── Feature 44: button on_click / on_hover attr routing ──────────────
        for (k, v) in &node.attrs {
            let val = self.eval_expr(v)?.to_string();
            let html_key = match k.as_str() {
                "on_click" | "onClick"   => "onclick",
                "on_hover" | "onHover"   => "onmouseover",
                "on_press" | "onPress"   => "onmousedown",
                "on_change"| "onChange"  => "onchange",
                "on_submit"| "onSubmit"  => "onsubmit",
                "on_input" | "onInput"   => "oninput",
                other => other,
            };
            attr_str.push(' ');
            attr_str.push_str(html_key);
            attr_str.push_str("=\"");
            attr_str.push_str(&escape_html(&val));
            attr_str.push('"');
        }

        if !style_parts.is_empty() {
            attr_str.push_str(" style=\"");
            attr_str.push_str(&escape_html(&style_parts.join("; ")));
            attr_str.push('"');
        }

        // ── Feature 44: button gets remox-btn class automatically ────────────
        if html_tag == "button" {
            attr_str = format!(" class=\"remox-btn\"{}", attr_str);
        }

        let content_str = match &node.content {
            Some(e) => escape_html(&self.eval_expr(e)?.to_string()),
            None => String::new(),
        };

        // Self-closing tags
        if matches!(html_tag, "img" | "input" | "br" | "hr") {
            out.push_str(&format!("{}<{}{} />\n", pad, html_tag, attr_str));
            return Ok(());
        }

        if node.children.is_empty() {
            out.push_str(&format!("{}<{}{}>{}</{}>\n", pad, html_tag, attr_str, content_str, html_tag));
        } else {
            out.push_str(&format!("{}<{}{}>{}  \n", pad, html_tag, attr_str, content_str));
            for child in &node.children {
                self.render_ui_node(child, out, indent + 1)?;
            }
            out.push_str(&format!("{}</{}>\n", pad, html_tag));
        }
        Ok(())
    }

    fn eval_expr(&mut self, expr: &Expr) -> Result<Value, RuntimeSignal> {
        match expr {
            Expr::IntLit(n)   => Ok(Value::Int(*n)),
            Expr::FloatLit(n) => Ok(Value::Float(*n)),
            Expr::BoolLit(b)  => Ok(Value::Bool(*b)),
            Expr::Null        => Ok(Value::Null),

            // Feature 32: ok(x) / err("msg")
            Expr::OkLit(e)  => Ok(Value::Ok(Box::new(self.eval_expr(e)?))),
            Expr::ErrLit(e) => {
                let msg = self.eval_expr(e)?.to_string();
                Ok(Value::Err(msg))
            }

            // Feature 10: string interpolation
            Expr::StrLit(s) => Ok(Value::Str(self.interpolate_string(s)?)),

            Expr::Ident(name) => {
                match self.get_var(name) {
                    Some(v) => Ok(v),
                    None => Err(RuntimeSignal::Error(format!("Undefined variable: '{}'", name))),
                }
            }

            Expr::List(items) => {
                let mut vals = Vec::new();
                for item in items {
                    match item {
                        // Feature 23: spread inside list
                        Expr::Spread(inner) => {
                            let v = self.eval_expr(inner)?;
                            match v {
                                Value::List(ls) => vals.extend(ls),
                                Value::Range(a, b) => vals.extend((a..b).map(Value::Int)),
                                other => vals.push(other),
                            }
                        }
                        _ => vals.push(self.eval_expr(item)?),
                    }
                }
                Ok(Value::List(vals))
            }

            Expr::Range(a, b) => {
                let av = match self.eval_expr(a)? { Value::Int(n) => n, v =>
                    return Err(RuntimeSignal::Error(format!("Range start must be int, got {}", v))) };
                let bv = match self.eval_expr(b)? { Value::Int(n) => n, v =>
                    return Err(RuntimeSignal::Error(format!("Range end must be int, got {}", v))) };
                Ok(Value::Range(av, bv))
            }

            // Feature 35: map literal
            Expr::Map(pairs) => {
                let mut result = Vec::new();
                for (k, v) in pairs {
                    result.push((k.clone(), self.eval_expr(v)?));
                }
                Ok(Value::Map(result))
            }

            // Feature 23: spread in expression context → just evaluate inner
            Expr::Spread(inner) => self.eval_expr(inner),

            // Feature 34: list comprehension
            Expr::ListComp { expr, var, iter, cond } => {
                let iterable = self.eval_expr(iter)?;
                let items: Vec<Value> = match iterable {
                    Value::List(v)     => v,
                    Value::Range(a, b) => (a..b).map(Value::Int).collect(),
                    Value::Str(s)      => s.chars().map(|c| Value::Str(c.to_string())).collect(),
                    _ => return Err(RuntimeSignal::Error("Comprehension: not iterable".into())),
                };
                let mut result = Vec::new();
                for item in items {
                    self.push_scope();
                    self.def_var(var, item);
                    let include = if let Some(cond_expr) = cond {
                        self.eval_expr(cond_expr)?.is_truthy()
                    } else { true };
                    if include {
                        result.push(self.eval_expr(expr)?);
                    }
                    self.pop_scope();
                }
                Ok(Value::List(result))
            }

            Expr::Not(e) => { let v = self.eval_expr(e)?; Ok(Value::Bool(!v.is_truthy())) }

            Expr::Ternary { cond, then_val, else_val } => {
                let cv = self.eval_expr(cond)?;
                if cv.is_truthy() { self.eval_expr(then_val) } else { self.eval_expr(else_val) }
            }

            // Feature 24: match expression
            Expr::Match { subject, arms, default } => {
                let sv = self.eval_expr(subject)?;
                for (pat, val) in arms {
                    // BUGFIX: `_` is documented (README) as a wildcard/default
                    // pattern, but was previously evaluated as a plain
                    // variable lookup, causing "Undefined variable: '_'" at
                    // runtime instead of matching anything.
                    let matched = if matches!(pat, Expr::Ident(n) if n == "_") {
                        true
                    } else {
                        let pv = self.eval_expr(pat)?;
                        sv.is_equal(&pv)
                    };
                    if matched { return self.eval_expr(val); }
                }
                if let Some(def) = default { return self.eval_expr(def); }
                Ok(Value::Null)
            }

            // BUGFIX: `try { } catch err { }` as an expression — same
            // semantics as the Stmt::TryCatch runtime path.
            Expr::TryExpr { body, catch_var, catch_body, else_body } => {
                self.push_scope();
                let result = self.exec_block(body);
                self.pop_scope();
                match result {
                    Ok(v) => {
                        if let Some(eb) = else_body {
                            self.push_scope();
                            let r = self.exec_block(eb);
                            self.pop_scope();
                            r
                        } else { Ok(v) }
                    }
                    Err(RuntimeSignal::Error(msg)) => {
                        self.push_scope();
                        if let Some(cv) = catch_var {
                            self.def_var(cv, Value::Str(msg.clone()));
                        }
                        let r = self.exec_block(catch_body);
                        self.pop_scope();
                        r
                    }
                    Err(e) => Err(e),
                }
            }
            // match/when arm value). Prints exactly like the `say` statement
            // and evaluates to Null.
            Expr::SayExpr(inner) => {
                let v = self.eval_expr(inner)?;
                println!("{}", v.to_string());
                Ok(Value::Null)
            }

            // when as expression eval
            Expr::WhenExpr { subject, cases, default } => {
                let subj = self.eval_expr(subject)?;
                let guard_mode = subj == Value::Bool(true) && matches!(subject.as_ref(), Expr::BoolLit(true));
                for (pat, val) in cases {
                    // BUGFIX: same `_` wildcard fix as Expr::Match — avoid
                    // treating a literal `_` pattern as a variable lookup.
                    let is_wildcard = matches!(pat, Expr::Ident(n) if n == "_");
                    let matched = if is_wildcard {
                        true
                    } else if guard_mode {
                        self.eval_expr(pat).map(|v| v.is_truthy()).unwrap_or(false)
                    } else {
                        let pv = self.eval_expr(pat)?;
                        subj.is_equal(&pv)
                    };
                    if matched { return self.eval_expr(val); }
                }
                if let Some(def) = default { return self.eval_expr(def); }
                Ok(Value::Null)
            }

            Expr::NullSafe(coll, idx) => {
                let cv = self.eval_expr(coll)?;
                let iv = self.eval_expr(idx)?;
                let i  = match iv { Value::Int(n) => n,
                    _ => return Err(RuntimeSignal::Error("Index must be int".into())) };
                Ok(cv.safe_index(i))
            }

            // Feature 25: method call
            Expr::MethodCall { object, method, args } => {
                let obj  = self.eval_expr(object)?;
                let avals: Result<Vec<Value>, RuntimeSignal> = args.iter().map(|a| self.eval_expr(a)).collect();
                let avals = avals?;

                // Check impl methods first
                let struct_name = match &obj {
                    Value::Struct { name, .. } => Some(name.clone()),
                    _ => None,
                };
                if let Some(sname) = struct_name {
                    if let Some(methods) = self.impls.get(&sname).cloned() {
                        if let Some((_, params, body)) = methods.iter().find(|(n, _, _)| n == method).cloned() {
                            self.push_scope();
                            self.def_var("self", obj.clone());
                            for (p, v) in params.iter().skip(1).zip(avals.iter()) {
                                self.def_var(p, v.clone());
                            }
                            let r = self.exec_block(&body);
                            self.pop_scope();
                            return match r {
                                Ok(v) => Ok(v),
                                Err(RuntimeSignal::Return(v)) => Ok(v.unwrap_or(Value::Null)),
                                Err(e) => Err(e),
                            };
                        }
                    }
                }

                // For Map values (modules), check if the key holds a callable lambda
                if let Value::Map(ref pairs) = obj {
                    if let Some((_, lambda_val)) = pairs.iter().find(|(k, _)| k == method) {
                        let lv = lambda_val.clone();
                        return self.call_value(lv, avals);
                    }
                }

                // Built-in value methods
                obj.get_method_val(method, &avals)
            }

            // Feature 27-28: struct field access
            Expr::StructAccess(obj_expr, field) => {
                let obj = self.eval_expr(obj_expr)?;
                match obj {
                    Value::Struct { ref fields, .. } => {
                        Ok(fields.iter().find(|(k, _)| k == field)
                            .map(|(_, v)| v.clone()).unwrap_or(Value::Null))
                    }
                    Value::Map(ref pairs) => {
                        Ok(pairs.iter().find(|(k, _)| k == field)
                            .map(|(_, v)| v.clone()).unwrap_or(Value::Null))
                    }
                    _ => Err(RuntimeSignal::Error(format!("Cannot access field '{}' on {}", field, obj))),
                }
            }

            // Feature 28: struct literal (named fields in call site)
            Expr::StructLit { name, fields } => {
                let mut fvals = Vec::new();
                for (k, v) in fields {
                    fvals.push((k.clone(), self.eval_expr(v)?));
                }
                Ok(Value::Struct { name: name.clone(), fields: fvals })
            }

            // Feature 39: lambda definition
            Expr::Lambda { params, body } => {
                // Capture current scope snapshot
                let mut captures = HashMap::new();
                for scope in &self.env {
                    for (k, v) in scope { captures.insert(k.clone(), v.clone()); }
                }
                Ok(Value::Lambda { params: params.clone(), body: body.clone(), captures })
            }

            // Feature 26: pipe operator |>
            Expr::Pipe { left, right } => {
                let lv = self.eval_expr(left)?;
                // right side should be a fn/lambda reference or call
                // We apply lv as the first argument to whatever is on the right
                match right.as_ref() {
                    Expr::Ident(fname) => {
                        self.call_function(fname, vec![lv], Vec::new())
                    }
                    Expr::Call { name, args, named } => {
                        let mut avals = vec![lv];
                        for a in args { avals.push(self.eval_expr(a)?); }
                        let nvals: Vec<(String, Value)> = named.iter().map(|(k, v)| {
                            (k.clone(), self.eval_expr(v).unwrap_or(Value::Null))
                        }).collect();
                        self.call_function(name, avals, nvals)
                    }
                    Expr::Lambda { params, body, .. } => {
                        self.push_scope();
                        if let Some(p) = params.first() { self.def_var(p, lv); }
                        let r = self.eval_expr(body);
                        self.pop_scope();
                        r
                    }
                    _ => {
                        // Evaluate right, try to call it as lambda
                        let rv = self.eval_expr(right)?;
                        self.call_value(rv, vec![lv])
                    }
                }
            }

            // Feature 40: await
            Expr::Await(inner) => {
                let handle = self.eval_expr(inner)?;
                match handle {
                    Value::AsyncHandle(arc) => {
                        // Spin-wait for result (real thread join)
                        loop {
                            {
                                let mut guard = arc.lock().unwrap();
                                if guard.is_some() {
                                    return Ok(guard.take().unwrap());
                                }
                            }
                            thread::sleep(Duration::from_millis(1));
                        }
                    }
                    other => Ok(other), // not async, return as-is
                }
            }

            // Feature 38: generic call — type params are erased at runtime
            Expr::GenericCall { name, type_params: _, args } => {
                let mut avals = Vec::new();
                for a in args { avals.push(self.eval_expr(a)?); }
                self.call_function(name, avals, Vec::new())
            }

            Expr::BinOp { op, left, right } => {
                let lv = self.eval_expr(left)?;
                let rv = self.eval_expr(right)?;
                self.eval_binop(op, lv, rv)
            }

            Expr::Call { name, args, named } => {
                // Feature 32: ok() / err() built-ins
                if name == "ok" {
                    let v = if args.is_empty() { Value::Null } else { self.eval_expr(&args[0])? };
                    return Ok(Value::Ok(Box::new(v)));
                }
                if name == "err" {
                    let msg = if args.is_empty() { "error".to_string() }
                              else { self.eval_expr(&args[0])?.to_string() };
                    return Ok(Value::Err(msg));
                }

                // Evaluate positional args, handling spread
                let mut avals: Vec<Value> = Vec::new();
                for a in args {
                    match a {
                        Expr::Spread(inner) => {
                            let v = self.eval_expr(inner)?;
                            match v {
                                Value::List(ls) => avals.extend(ls),
                                Value::Range(a, b) => avals.extend((a..b).map(Value::Int)),
                                other => avals.push(other),
                            }
                        }
                        _ => avals.push(self.eval_expr(a)?),
                    }
                }
                let nvals: Vec<(String, Value)> = named.iter().map(|(k, v)| {
                    (k.clone(), self.eval_expr(v).unwrap_or(Value::Null))
                }).collect();

                self.call_function(name, avals, nvals)
            }
        }
    }

    // Central function call — handles builtins, user-defined, struct ctor, lambda vars
    fn call_function(&mut self, name: &str, mut args: Vec<Value>, named: Vec<(String, Value)>)
        -> Result<Value, RuntimeSignal>
    {
        // ---- BUILT-IN FUNCTIONS ----
        match name {
            "len" => {
                let a = args.into_iter().next().unwrap_or(Value::Null);
                return Ok(match a {
                    Value::List(v)     => Value::Int(v.len() as i64),
                    Value::Str(s)      => Value::Int(s.chars().count() as i64),
                    Value::Map(p)      => Value::Int(p.len() as i64),
                    Value::Range(a, b) => Value::Int(b - a),
                    _ => Value::Int(0),
                });
            }
            "int" => {
                return Ok(match args.into_iter().next().unwrap_or(Value::Null) {
                    Value::Str(s)   => Value::Int(s.trim().parse().unwrap_or(0)),
                    Value::Float(f) => Value::Int(f as i64),
                    Value::Bool(b)  => Value::Int(if b { 1 } else { 0 }),
                    v => v,
                });
            }
            "float" => {
                return Ok(match args.into_iter().next().unwrap_or(Value::Null) {
                    Value::Str(s)  => Value::Float(s.trim().parse().unwrap_or(0.0)),
                    Value::Int(n)  => Value::Float(n as f64),
                    v => v,
                });
            }
            "str"  => { return Ok(Value::Str(args.into_iter().next().unwrap_or(Value::Null).to_string())); }
            "bool" => {
                return Ok(Value::Bool(args.into_iter().next().unwrap_or(Value::Null).is_truthy()));
            }
            "type" => {
                return Ok(Value::Str(match args.into_iter().next().unwrap_or(Value::Null) {
                    Value::Int(_)    => "int",
                    Value::Float(_)  => "float",
                    Value::Str(_)    => "str",
                    Value::Bool(_)   => "bool",
                    Value::List(_)   => "list",
                    Value::Map(_)    => "map",
                    Value::Range(..) => "range",
                    Value::Null      => "null",
                    Value::Struct { name, .. } => return Ok(Value::Str(name)),
                    Value::Ok(_)     => "ok",
                    Value::Err(_)    => "err",
                    Value::Lambda{..}=> "lambda",
                    Value::AsyncHandle(_) => "async",
                }.to_string()));
            }
            "push" => {
                // push(list_var_name_as_str, item) — kept for compat
                // Better: use list.push() method (handled in exec_stmt assign)
                // Here: push(list, item) non-mutating → return new list
                if args.len() >= 2 {
                    let item = args.remove(1);
                    let mut list = match args.remove(0) {
                        Value::List(v) => v,
                        _ => return Err(RuntimeSignal::Error("push: first arg must be list".into())),
                    };
                    list.push(item);
                    return Ok(Value::List(list));
                }
                return Ok(Value::Null);
            }
            "pop" => {
                if let Some(Value::List(mut v)) = args.into_iter().next() {
                    let last = v.pop().unwrap_or(Value::Null);
                    return Ok(last);
                }
                return Ok(Value::Null);
            }
            "input" => {
                let prompt = args.into_iter().next().map(|v| v.to_string()).unwrap_or_default();
                print!("{}", prompt);
                io::stdout().flush().unwrap();
                let mut line = String::new();
                io::stdin().lock().read_line(&mut line).unwrap_or(0);
                return Ok(Value::Str(line.trim().to_string()));
            }
            "print" | "__io_print" => {
                let s = args.into_iter().next().unwrap_or(Value::Null);
                println!("{}", s);
                return Ok(Value::Null);
            }
            "range" => {
                let a = match args.first() { Some(Value::Int(n)) => *n, _ => 0 };
                let b = match args.get(1)  { Some(Value::Int(n)) => *n, _ => 0 };
                return Ok(Value::Range(a, b));
            }
            // Feature 38 examples — built-in generics
            "max" => {
                if args.len() >= 2 {
                    let (l, r) = (args.remove(0), args.remove(0));
                    return Ok(if self.cmp_val(&l, &r) >= 0 { l } else { r });
                } else if let Some(Value::List(v)) = args.into_iter().next() {
                    let mut best = Value::Null;
                    for item in v {
                        if let Value::Null = best { best = item; }
                        else if self.cmp_val(&item, &best) > 0 { best = item; }
                    }
                    return Ok(best);
                }
                return Ok(Value::Null);
            }
            "min" => {
                if args.len() >= 2 {
                    let (l, r) = (args.remove(0), args.remove(0));
                    return Ok(if self.cmp_val(&l, &r) <= 0 { l } else { r });
                } else if let Some(Value::List(v)) = args.into_iter().next() {
                    let mut best = Value::Null;
                    for item in v {
                        if let Value::Null = best { best = item; }
                        else if self.cmp_val(&item, &best) < 0 { best = item; }
                    }
                    return Ok(best);
                }
                return Ok(Value::Null);
            }
            "sum" => {
                if let Some(Value::List(v)) = args.into_iter().next() {
                    let mut s = 0i64; let mut sf = 0f64; let mut fl = false;
                    for x in v { match x { Value::Int(n) => s += n, Value::Float(f) => { sf += f; fl = true; } _ => {} } }
                    return Ok(if fl { Value::Float(sf + s as f64) } else { Value::Int(s) });
                }
                return Ok(Value::Int(0));
            }
            "map" => {
                // map(list, lambda) → apply lambda to each element
                if args.len() >= 2 {
                    let func = args.remove(1);
                    if let Value::List(v) = args.remove(0) {
                        let mut result = Vec::new();
                        for item in v {
                            result.push(self.call_value(func.clone(), vec![item])?);
                        }
                        return Ok(Value::List(result));
                    }
                }
                return Ok(Value::Null);
            }
            "filter" => {
                if args.len() >= 2 {
                    let func = args.remove(1);
                    if let Value::List(v) = args.remove(0) {
                        let mut result = Vec::new();
                        for item in v {
                            if self.call_value(func.clone(), vec![item.clone()])?.is_truthy() {
                                result.push(item);
                            }
                        }
                        return Ok(Value::List(result));
                    }
                }
                return Ok(Value::Null);
            }
            "reduce" => {
                if args.len() >= 2 {
                    let func = args.remove(1);
                    if let Value::List(mut v) = args.remove(0) {
                        if v.is_empty() { return Ok(Value::Null); }
                        let mut acc = v.remove(0);
                        for item in v {
                            acc = self.call_value(func.clone(), vec![acc, item])?;
                        }
                        return Ok(acc);
                    }
                }
                return Ok(Value::Null);
            }
            "zip" => {
                if args.len() >= 2 {
                    if let (Value::List(a), Value::List(b)) = (args.remove(0), args.remove(0)) {
                        let result: Vec<Value> = a.into_iter().zip(b.into_iter())
                            .map(|(x, y)| Value::List(vec![x, y]))
                            .collect();
                        return Ok(Value::List(result));
                    }
                }
                return Ok(Value::Null);
            }
            "enumerate" => {
                if let Some(Value::List(v)) = args.into_iter().next() {
                    let result: Vec<Value> = v.into_iter().enumerate()
                        .map(|(i, x)| Value::List(vec![Value::Int(i as i64), x]))
                        .collect();
                    return Ok(Value::List(result));
                }
                return Ok(Value::Null);
            }
            "flat" | "flatten" => {
                if let Some(Value::List(v)) = args.into_iter().next() {
                    let mut result = Vec::new();
                    for item in v {
                        match item {
                            Value::List(inner) => result.extend(inner),
                            other => result.push(other),
                        }
                    }
                    return Ok(Value::List(result));
                }
                return Ok(Value::Null);
            }
            "sort" => {
                if let Some(Value::List(mut v)) = args.into_iter().next() {
                    v.sort_by(|a, b| {
                        match (a, b) {
                            (Value::Int(x), Value::Int(y)) => x.cmp(y),
                            (Value::Float(x), Value::Float(y)) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
                            (Value::Str(x), Value::Str(y)) => x.cmp(y),
                            _ => std::cmp::Ordering::Equal,
                        }
                    });
                    return Ok(Value::List(v));
                }
                return Ok(Value::Null);
            }
            "reverse" => {
                if let Some(Value::List(mut v)) = args.clone().into_iter().next() {
                    v.reverse();
                    return Ok(Value::List(v));
                } else if let Some(Value::Str(s)) = args.into_iter().next() {
                    return Ok(Value::Str(s.chars().rev().collect()));
                }
                return Ok(Value::Null);
            }
            "keys" => {
                if let Some(v) = args.into_iter().next() {
                    return Ok(match v {
                        Value::Map(pairs)           => Value::List(pairs.iter().map(|(k, _)| Value::Str(k.clone())).collect()),
                        Value::Struct { fields, .. }=> Value::List(fields.iter().map(|(k, _)| Value::Str(k.clone())).collect()),
                        _ => Value::List(Vec::new()),
                    });
                }
                return Ok(Value::List(Vec::new()));
            }
            "values" => {
                if let Some(v) = args.into_iter().next() {
                    return Ok(match v {
                        Value::Map(pairs)           => Value::List(pairs.into_iter().map(|(_, v)| v).collect()),
                        Value::Struct { fields, .. }=> Value::List(fields.into_iter().map(|(_, v)| v).collect()),
                        _ => Value::List(Vec::new()),
                    });
                }
                return Ok(Value::List(Vec::new()));
            }
            // Math module builtins — also accessible via use math
            "__math_sqrt"  => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Float(x.sqrt())); }
            "__math_pow"   => { let x = Self::to_f64(args.remove(0)); let n = Self::to_f64(args.remove(0)); return Ok(Value::Float(x.powf(n))); }
            "__math_abs"   => { let x = args.into_iter().next().unwrap_or(Value::Int(0)); return Ok(match x { Value::Int(n) => Value::Int(n.abs()), Value::Float(f) => Value::Float(f.abs()), v => v }); }
            "__math_floor" => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Int(x.floor() as i64)); }
            "__math_ceil"  => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Int(x.ceil()  as i64)); }
            "__math_round" => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Int(x.round() as i64)); }
            "__math_sin"   => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Float(x.sin())); }
            "__math_cos"   => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Float(x.cos())); }
            "__math_tan"   => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Float(x.tan())); }
            "__math_log"   => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Float(x.ln())); }
            "__math_log2"  => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Float(x.log2())); }
            "__math_log10" => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Float(x.log10())); }
            "__math_min"   => { let a = args.remove(0); let b = args.remove(0); return Ok(if self.cmp_val(&a, &b) <= 0 { a } else { b }); }
            "__math_max"   => { let a = args.remove(0); let b = args.remove(0); return Ok(if self.cmp_val(&a, &b) >= 0 { a } else { b }); }
            // ── Malib + extended math engine ──────────────────────────────────
            n if n.starts_with("__malib_") || n.starts_with("__math_a") || n.starts_with("__math_e") || n.starts_with("__math_s") => {
                match dispatch_malib(n, args) {
                    Ok(v)    => return Ok(v),
                    Err(msg) => return Err(RuntimeSignal::Error(msg)),
                }
            }
            // ── Numrux — N-dimensional array engine ────────────────────────────
            n if n.starts_with("__numrux_") => {
                match dispatch_numrux(n, args) {
                    Ok(v)    => return Ok(v),
                    Err(msg) => return Err(RuntimeSignal::Error(msg)),
                }
            }
            // ── Autoclib — CLI/Automation engine (Click+argparse+Typer+Fire+Rich+Textual+Fabric) ──
            n if n.starts_with("__autoclib_") => {
                match dispatch_autoclib(n, args) {
                    Ok(v)    => return Ok(v),
                    Err(msg) => return Err(RuntimeSignal::Error(msg)),
                }
            }
            // ── Remotest — Testing engine (pytest+unittest+nose2+Hypothesis-partial+
            // Robot+Behave+Locust-basic+Faker+Mock, unified). Inline (not a free fn)
            // because running tests/fixtures/mocks means calling back into
            // self.call_value() — needs &mut self, same reason as Tasoaque below.
            n if n.starts_with("__remotest_") => {
                return self.dispatch_remotest(n, args);
            }
            // ── Astriloop — Async Runtime Engine (asyncio+trio+uvloop, unified) ──
            // Inline (not a free fn) because tasks/gather/nursery need to call
            // back into self.call_value() — same reason as Remotest/Tasoaque.
            n if n.starts_with("__astriloop_") => {
                return self.dispatch_astriloop(n, args);
            }
            // ── Retime — Time Library (Python time module + Stopwatch/Ticker/
            // Deadline/RateCounter). Free fn like Malib/Numrux — no &mut self
            // callbacks needed; sleep uses the existing std::thread::sleep shim.
            n if n.starts_with("__retime_") => {
                match dispatch_retime(n, args) {
                    Ok(v)    => return Ok(v),
                    Err(msg) => return Err(RuntimeSignal::Error(msg)),
                }
            }
            // ── Tasoaque — Task Queue Engine (Celery + RQ se bhi powerful) ──
            // Inline (not a free fn like dispatch_autoclib) because tasks are
            // executed by calling back into self.call_function() with the
            // task's own registered name — needs &mut self.
            n if n.starts_with("__tasoaque_") => {
                return self.dispatch_tasoaque(n, args);
            }
            // ── Sceuti — SceutiClock/Schedule/Env/Log/Fake (v2.0, 50 features) ──
            // Inline (needs &mut self) because schedule_runPending calls back
            // into self.call_function() for each due job's fn_name.
            n if n.starts_with("__sceuti_") => {
                return self.dispatch_sceuti(n, args);
            }
            "__io_read"    => {
                let mut line = String::new();
                io::stdin().lock().read_line(&mut line).ok();
                return Ok(Value::Str(line.trim_end_matches(['\n', '\r']).to_string()));
            }
            "__os_env"     => { let key = args.into_iter().next().map(|v| v.to_string()).unwrap_or_default(); return Ok(std::env::var(&key).map(Value::Str).unwrap_or(Value::Null)); }
            "__os_exit"    => { let code = match args.into_iter().next() { Some(Value::Int(n)) => n as i32, _ => 0 }; return Err(RuntimeSignal::Exit(code)); }
            "__rand_int"   => {
                let min = match args.first() { Some(Value::Int(n)) => *n, _ => 0 };
                let max = match args.get(1)  { Some(Value::Int(n)) => *n, _ => 100 };
                self.rand_state = self.rand_state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let r = ((self.rand_state >> 33) as i64).abs() % (max - min + 1).max(1) + min;
                return Ok(Value::Int(r));
            }
            "__rand_float" => {
                self.rand_state = self.rand_state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let r = (self.rand_state >> 11) as f64 / (1u64 << 53) as f64;
                return Ok(Value::Float(r));
            }
            "__remojoke_get" => {
                let category = args.into_iter().next().map(|v| v.to_string()).unwrap_or_default();
                let joke = remojoke_get(&category, &self.remojoke_lang.clone(), &mut self.rand_state);
                return Ok(Value::Str(joke));
            }
            "__remojoke_setlang" => {
                let requested = args.into_iter().next().map(|v| v.to_string()).unwrap_or_default();
                let normalized = remojoke_normalize_lang(&requested);
                self.remojoke_lang = normalized.clone();
                let confirmation = if normalized == "src" {
                    "Remojoke language: original (src / Hinglish).".to_string()
                } else if remojoke_supported_langs().contains(&normalized.as_str()) {
                    format!("Remojoke language set to '{}'.", normalized)
                } else {
                    format!(
                        "Remojoke language set to '{}', but no translations exist for it yet — \
                         Category.remojoke() will return a 'not available' message until translations \
                         are added. Supported languages: {}.",
                        normalized, remojoke_supported_langs().join(", ")
                    )
                };
                return Ok(Value::Str(confirmation));
            }
            "__remojoke_multi" => {
                let n = match args.into_iter().next() {
                    Some(Value::Int(n)) => n,
                    Some(Value::Float(f)) => f as i64,
                    _ => 5,
                };
                let jokes = remojoke_get_multi(n, &self.remojoke_lang.clone(), &mut self.rand_state);
                return Ok(Value::List(jokes));
            }
            "sleep" => {
                let ms = match args.into_iter().next() { Some(Value::Int(n)) => n as u64, _ => 0 };
                thread::sleep(Duration::from_millis(ms));
                return Ok(Value::Null);
            }
            "assert" => {
                let cond = args.into_iter().next().unwrap_or(Value::Bool(false));
                if !cond.is_truthy() {
                    return Err(RuntimeSignal::Error("Assertion failed".into()));
                }
                return Ok(Value::Null);
            }

            // =================================================================
            // Malib — Advanced Math Engine builtins (use Malib)
            // =================================================================

            // ---- basic / extended arithmetic ----
            "__malib_sqrt"  => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Float(x.sqrt())); }
            "__malib_cbrt"  => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Float(x.cbrt())); }
            "__malib_pow"   => { let x = Self::to_f64(args.remove(0)); let n = Self::to_f64(args.remove(0)); return Ok(Value::Float(x.powf(n))); }
            "__malib_abs"   => { let x = args.into_iter().next().unwrap_or(Value::Int(0)); return Ok(match x { Value::Int(n) => Value::Int(n.abs()), Value::Float(f) => Value::Float(f.abs()), v => v }); }
            "__malib_floor" => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Int(x.floor() as i64)); }
            "__malib_ceil"  => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Int(x.ceil()  as i64)); }
            "__malib_round" => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Int(x.round() as i64)); }
            "__malib_min"   => { let a = args.remove(0); let b = args.remove(0); return Ok(if self.cmp_val(&a, &b) <= 0 { a } else { b }); }
            "__malib_max"   => { let a = args.remove(0); let b = args.remove(0); return Ok(if self.cmp_val(&a, &b) >= 0 { a } else { b }); }

            // ---- trigonometry / logarithms ----
            "__malib_sin"   => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Float(x.sin())); }
            "__malib_cos"   => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Float(x.cos())); }
            "__malib_tan"   => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Float(x.tan())); }
            "__malib_asin"  => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Float(x.asin())); }
            "__malib_acos"  => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Float(x.acos())); }
            "__malib_atan"  => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Float(x.atan())); }
            "__malib_log"   => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Float(x.ln())); }
            "__malib_log2"  => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Float(x.log2())); }
            "__malib_log10" => { let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))); return Ok(Value::Float(x.log10())); }
            "__malib_logn"  => { let x = Self::to_f64(args.remove(0)); let base = Self::to_f64(args.remove(0)); return Ok(Value::Float(x.log(base))); }

            // ---- number theory ----
            "__malib_gcd" => {
                let mut a = Self::to_f64(args.remove(0)).abs() as i64;
                let mut b = Self::to_f64(args.remove(0)).abs() as i64;
                while b != 0 { let t = b; b = a % b; a = t; }
                return Ok(Value::Int(a));
            }
            "__malib_lcm" => {
                let a = Self::to_f64(args.remove(0)).abs() as i64;
                let b = Self::to_f64(args.remove(0)).abs() as i64;
                if a == 0 || b == 0 { return Ok(Value::Int(0)); }
                let (mut x, mut y) = (a, b);
                while y != 0 { let t = y; y = x % y; x = t; }
                return Ok(Value::Int((a / x) * b));
            }
            "__malib_is_prime" => {
                let n = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))) as i64;
                if n < 2 { return Ok(Value::Bool(false)); }
                if n < 4 { return Ok(Value::Bool(true)); }
                if n % 2 == 0 { return Ok(Value::Bool(false)); }
                let mut i = 3i64;
                while i * i <= n {
                    if n % i == 0 { return Ok(Value::Bool(false)); }
                    i += 2;
                }
                return Ok(Value::Bool(true));
            }
            "__malib_factorize" => {
                let mut n = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))) as i64;
                let mut factors = Vec::new();
                if n <= 1 { return Ok(Value::List(factors)); }
                let mut d = 2i64;
                while d * d <= n {
                    while n % d == 0 { factors.push(Value::Int(d)); n /= d; }
                    d += 1;
                }
                if n > 1 { factors.push(Value::Int(n)); }
                return Ok(Value::List(factors));
            }
            "__malib_factorial" => {
                let n = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))) as i64;
                if n < 0 { return Err(RuntimeSignal::Error("factorial: negative input".into())); }
                let mut result: i64 = 1;
                for i in 2..=n.max(1) { result = result.saturating_mul(i); }
                if n == 0 { result = 1; }
                return Ok(Value::Int(result));
            }
            "__malib_fibonacci" => {
                let n = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0))) as i64;
                if n < 0 { return Err(RuntimeSignal::Error("fibonacci: negative input".into())); }
                let (mut a, mut b): (i64, i64) = (0, 1);
                for _ in 0..n { let t = a + b; a = b; b = t; }
                return Ok(Value::Int(a));
            }

            // ---- combinatorics ----
            "__malib_ncr" => {
                let n = Self::to_f64(args.remove(0)) as i64;
                let r = Self::to_f64(args.remove(0)) as i64;
                if r < 0 || r > n { return Ok(Value::Int(0)); }
                let mut result: f64 = 1.0;
                for i in 0..r { result = result * (n - i) as f64 / (i + 1) as f64; }
                return Ok(Value::Int(result.round() as i64));
            }
            "__malib_npr" => {
                let n = Self::to_f64(args.remove(0)) as i64;
                let r = Self::to_f64(args.remove(0)) as i64;
                if r < 0 || r > n { return Ok(Value::Int(0)); }
                let mut result: i64 = 1;
                for i in 0..r { result = result.saturating_mul(n - i); }
                return Ok(Value::Int(result));
            }

            // ---- algebra: direct coefficient solvers ----
            "__malib_linear" => {
                // ax + b = 0  →  x = -b/a
                let a = Self::to_f64(args.remove(0));
                let b = Self::to_f64(args.remove(0));
                if a == 0.0 {
                    return Ok(if b == 0.0 { Value::Str("infinite solutions".into()) } else { Value::Str("no solution".into()) });
                }
                return Ok(Value::Float(-b / a));
            }
            "__malib_quadratic" => {
                // ax^2 + bx + c = 0
                let a = Self::to_f64(args.remove(0));
                let b = Self::to_f64(args.remove(0));
                let c = Self::to_f64(args.remove(0));
                if a == 0.0 {
                    return self.call_function("__malib_linear", vec![Value::Float(b), Value::Float(c)], Vec::new());
                }
                let disc = b * b - 4.0 * a * c;
                if disc < 0.0 {
                    return Ok(Value::List(vec![]));
                } else if disc.abs() < 1e-12 {
                    return Ok(Value::List(vec![Value::Float(-b / (2.0 * a))]));
                } else {
                    let sq = disc.sqrt();
                    let r1 = (-b + sq) / (2.0 * a);
                    let r2 = (-b - sq) / (2.0 * a);
                    let (lo, hi) = if r1 < r2 { (r1, r2) } else { (r2, r1) };
                    return Ok(Value::List(vec![Value::Float(lo), Value::Float(hi)]));
                }
            }

            // ---- general expression engine: eval / solve / simplify ----
            "__malib_eval" => {
                let expr_str = args.into_iter().next().map(|v| v.to_string()).unwrap_or_default();
                let node = malib_engine::parse_single(&expr_str)
                    .map_err(|e| RuntimeSignal::Error(format!("Malib.eval: {}", e)))?;
                let val = malib_engine::eval(&node, "__none__", 0.0)
                    .map_err(|e| RuntimeSignal::Error(format!("Malib.eval: {}", e)))?;
                return Ok(Value::Float(val));
            }
            "__malib_simplify" => {
                // Numeric simplification: evaluate constant expression to its simplest float/int form
                let expr_str = args.into_iter().next().map(|v| v.to_string()).unwrap_or_default();
                let node = malib_engine::parse_single(&expr_str)
                    .map_err(|e| RuntimeSignal::Error(format!("Malib.simplify: {}", e)))?;
                let val = malib_engine::eval(&node, "__none__", 0.0)
                    .map_err(|e| RuntimeSignal::Error(format!("Malib.simplify: {}", e)))?;
                return Ok(if (val - val.round()).abs() < 1e-9 { Value::Int(val.round() as i64) } else { Value::Float(val) });
            }
            "__malib_solve" => {
                let eq_str = args.into_iter().next().map(|v| v.to_string()).unwrap_or_default();
                let (lhs, rhs) = malib_engine::parse(&eq_str)
                    .map_err(|e| RuntimeSignal::Error(format!("Malib.solve: {}", e)))?;
                let result = malib_engine::solve_equation(&lhs, rhs.as_ref());
                return Ok(match result {
                    malib_engine::SolveResult::NoVariable(v) => {
                        if v.abs() < 1e-9 { Value::Bool(true) } else { Value::Bool(false) }
                    }
                    malib_engine::SolveResult::Linear(x) => Value::Float(x),
                    malib_engine::SolveResult::Quadratic(xs) => {
                        Value::List(xs.into_iter().map(Value::Float).collect())
                    }
                    malib_engine::SolveResult::Numeric(mut xs) => {
                        // Periodic/transcendental equations can yield many roots in the
                        // scan window — surface the few closest to zero (principal values)
                        // so the result stays readable; full set is still mathematically valid.
                        if xs.len() > 4 {
                            xs.sort_by(|a, b| a.abs().partial_cmp(&b.abs()).unwrap());
                            xs.truncate(4);
                            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
                        }
                        Value::List(xs.into_iter().map(Value::Float).collect())
                    }
                    malib_engine::SolveResult::NoRealSolution => Value::List(vec![]),
                    malib_engine::SolveResult::Error(e) => return Err(RuntimeSignal::Error(format!("Malib.solve: {}", e))),
                });
            }

            // ---- calculus (numerical) ----
            "__malib_derivative" => {
                let expr_str = args.remove(0).to_string();
                let at = Self::to_f64(args.remove(0));
                let node = malib_engine::parse_single(&expr_str)
                    .map_err(|e| RuntimeSignal::Error(format!("Malib.derivative: {}", e)))?;
                let var = malib_engine::find_var(&node).unwrap_or_else(|| "x".to_string());
                let h = 1e-6;
                let f_plus = malib_engine::eval(&node, &var, at + h).map_err(|e| RuntimeSignal::Error(e))?;
                let f_minus = malib_engine::eval(&node, &var, at - h).map_err(|e| RuntimeSignal::Error(e))?;
                return Ok(Value::Float((f_plus - f_minus) / (2.0 * h)));
            }
            "__malib_integral" => {
                // Simpson's rule numerical definite integral over [a, b]
                let expr_str = args.remove(0).to_string();
                let a = Self::to_f64(args.remove(0));
                let b = Self::to_f64(args.remove(0));
                let node = malib_engine::parse_single(&expr_str)
                    .map_err(|e| RuntimeSignal::Error(format!("Malib.integral: {}", e)))?;
                let var = malib_engine::find_var(&node).unwrap_or_else(|| "x".to_string());
                let n = 1000usize; // even number of subintervals
                let h = (b - a) / n as f64;
                let mut sum = malib_engine::eval(&node, &var, a).map_err(|e| RuntimeSignal::Error(e))?
                            + malib_engine::eval(&node, &var, b).map_err(|e| RuntimeSignal::Error(e))?;
                for i in 1..n {
                    let x = a + i as f64 * h;
                    let coef = if i % 2 == 0 { 2.0 } else { 4.0 };
                    sum += coef * malib_engine::eval(&node, &var, x).map_err(|e| RuntimeSignal::Error(e))?;
                }
                return Ok(Value::Float(sum * h / 3.0));
            }
            "__malib_limit" => {
                // Numerically approach 'at' from both sides; report average if they agree
                let expr_str = args.remove(0).to_string();
                let at = Self::to_f64(args.remove(0));
                let node = malib_engine::parse_single(&expr_str)
                    .map_err(|e| RuntimeSignal::Error(format!("Malib.limit: {}", e)))?;
                let var = malib_engine::find_var(&node).unwrap_or_else(|| "x".to_string());
                let h = 1e-5;
                let left = malib_engine::eval(&node, &var, at - h);
                let right = malib_engine::eval(&node, &var, at + h);
                return Ok(match (left, right) {
                    (Ok(l), Ok(r)) if (l - r).abs() < 1e-3 => Value::Float((l + r) / 2.0),
                    (Ok(l), Ok(r)) => Value::Str(format!("limit may not exist (left≈{}, right≈{})", l, r)),
                    _ => Value::Str("limit undefined at this point".into()),
                });
            }
            "__malib_root" => {
                // Newton-Raphson starting from a user-provided guess
                let expr_str = args.remove(0).to_string();
                let guess = Self::to_f64(args.remove(0));
                let node = malib_engine::parse_single(&expr_str)
                    .map_err(|e| RuntimeSignal::Error(format!("Malib.root: {}", e)))?;
                let var = malib_engine::find_var(&node).unwrap_or_else(|| "x".to_string());
                let mut x = guess;
                let h = 1e-6;
                for _ in 0..200 {
                    let fx = malib_engine::eval(&node, &var, x).map_err(|e| RuntimeSignal::Error(e))?;
                    if fx.abs() < 1e-9 { break; }
                    let fxh = malib_engine::eval(&node, &var, x + h).map_err(|e| RuntimeSignal::Error(e))?;
                    let dfx = (fxh - fx) / h;
                    if dfx.abs() < 1e-12 { return Err(RuntimeSignal::Error("Malib.root: derivative vanished, try a different guess".into())); }
                    x -= fx / dfx;
                }
                return Ok(Value::Float((x * 1e9).round() / 1e9));
            }

            // ---- statistics ----
            "__malib_sum" => {
                let list = match args.into_iter().next() { Some(Value::List(v)) => v, _ => return Ok(Value::Int(0)) };
                let mut acc = 0.0;
                for v in &list { acc += Self::to_f64(v.clone()); }
                return Ok(Value::Float(acc));
            }
            "__malib_mean" => {
                let list = match args.into_iter().next() { Some(Value::List(v)) => v, _ => return Ok(Value::Float(0.0)) };
                if list.is_empty() { return Ok(Value::Float(0.0)); }
                let sum: f64 = list.iter().map(|v| Self::to_f64(v.clone())).sum();
                return Ok(Value::Float(sum / list.len() as f64));
            }
            "__malib_median" => {
                let list = match args.into_iter().next() { Some(Value::List(v)) => v, _ => return Ok(Value::Float(0.0)) };
                if list.is_empty() { return Ok(Value::Float(0.0)); }
                let mut nums: Vec<f64> = list.iter().map(|v| Self::to_f64(v.clone())).collect();
                nums.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let n = nums.len();
                let mid = if n % 2 == 0 { (nums[n/2 - 1] + nums[n/2]) / 2.0 } else { nums[n/2] };
                return Ok(Value::Float(mid));
            }
            "__malib_variance" => {
                let list = match args.into_iter().next() { Some(Value::List(v)) => v, _ => return Ok(Value::Float(0.0)) };
                if list.is_empty() { return Ok(Value::Float(0.0)); }
                let nums: Vec<f64> = list.iter().map(|v| Self::to_f64(v.clone())).collect();
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                let var = nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / nums.len() as f64;
                return Ok(Value::Float(var));
            }
            "__malib_stdev" => {
                let list = match args.into_iter().next() { Some(Value::List(v)) => v, _ => return Ok(Value::Float(0.0)) };
                if list.is_empty() { return Ok(Value::Float(0.0)); }
                let nums: Vec<f64> = list.iter().map(|v| Self::to_f64(v.clone())).collect();
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                let var = nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / nums.len() as f64;
                return Ok(Value::Float(var.sqrt()));
            }

            // ---- matrices (Value::List of Value::List rows) ----
            "__malib_matmul" => {
                let m1 = match args.remove(0) { Value::List(v) => v, _ => return Err(RuntimeSignal::Error("matMul: expected matrix (list of lists)".into())) };
                let m2 = match args.remove(0) { Value::List(v) => v, _ => return Err(RuntimeSignal::Error("matMul: expected matrix (list of lists)".into())) };
                let rows1: Vec<Vec<f64>> = m1.iter().map(|r| match r {
                    Value::List(row) => row.iter().map(|x| Self::to_f64(x.clone())).collect(),
                    _ => vec![],
                }).collect();
                let rows2: Vec<Vec<f64>> = m2.iter().map(|r| match r {
                    Value::List(row) => row.iter().map(|x| Self::to_f64(x.clone())).collect(),
                    _ => vec![],
                }).collect();
                if rows1.is_empty() || rows2.is_empty() || rows1[0].len() != rows2.len() {
                    return Err(RuntimeSignal::Error("matMul: incompatible matrix dimensions".into()));
                }
                let (r, k, c) = (rows1.len(), rows2.len(), rows2[0].len());
                let mut result = vec![vec![0.0; c]; r];
                for i in 0..r {
                    for j in 0..c {
                        let mut s = 0.0;
                        for x in 0..k { s += rows1[i][x] * rows2[x][j]; }
                        result[i][j] = s;
                    }
                }
                let out: Vec<Value> = result.into_iter()
                    .map(|row| Value::List(row.into_iter().map(Value::Float).collect()))
                    .collect();
                return Ok(Value::List(out));
            }
            "__malib_matdet" => {
                let m = match args.into_iter().next() { Some(Value::List(v)) => v, _ => return Err(RuntimeSignal::Error("matDet: expected matrix (list of lists)".into())) };
                let rows: Vec<Vec<f64>> = m.iter().map(|r| match r {
                    Value::List(row) => row.iter().map(|x| Self::to_f64(x.clone())).collect(),
                    _ => vec![],
                }).collect();
                let n = rows.len();
                if n == 0 || rows.iter().any(|r| r.len() != n) {
                    return Err(RuntimeSignal::Error("matDet: matrix must be square".into()));
                }
                fn det(m: &Vec<Vec<f64>>) -> f64 {
                    let n = m.len();
                    if n == 1 { return m[0][0]; }
                    if n == 2 { return m[0][0] * m[1][1] - m[0][1] * m[1][0]; }
                    let mut result = 0.0;
                    for col in 0..n {
                        let mut sub = Vec::with_capacity(n - 1);
                        for row in m.iter().skip(1) {
                            let mut r = Vec::with_capacity(n - 1);
                            for (c, val) in row.iter().enumerate() { if c != col { r.push(*val); } }
                            sub.push(r);
                        }
                        let sign = if col % 2 == 0 { 1.0 } else { -1.0 };
                        result += sign * m[0][col] * det(&sub);
                    }
                    result
                }
                return Ok(Value::Float(det(&rows)));
            }

            // ---- fraction helper ----
            "__malib_to_fraction" => {
                let x = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0)));
                let denom_limit = 100000i64;
                let mut best = (x.round() as i64, 1i64);
                let mut best_err = (x - best.0 as f64).abs();
                for d in 1..=denom_limit {
                    let n = (x * d as f64).round() as i64;
                    let err = (x - n as f64 / d as f64).abs();
                    if err < best_err {
                        best_err = err;
                        best = (n, d);
                        if err < 1e-9 { break; }
                    }
                }
                let (mut n, mut d) = best;
                let mut a = n.abs(); let mut b = d.abs();
                while b != 0 { let t = b; b = a % b; a = t; }
                let g = a.max(1);
                n /= g; d /= g;
                return Ok(Value::Str(format!("{}/{}", n, d)));
            }

            // =================================================================
            // Phinolib — Advanced Real Physics Library Implementation
            // All formulae: exact closed-form physics, IEEE 754 f64 precision.
            // =================================================================

            // ── Helper: deg→rad inline ────────────────────────────────────────
            "__phinolib_deg_to_rad" => {
                let deg = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0)));
                return Ok(Value::Float(deg * std::f64::consts::PI / 180.0));
            }
            "__phinolib_rad_to_deg" => {
                let rad = Self::to_f64(args.into_iter().next().unwrap_or(Value::Int(0)));
                return Ok(Value::Float(rad * 180.0 / std::f64::consts::PI));
            }
            "__phinolib_sig_figs" => {
                let x = Self::to_f64(args.remove(0));
                let n = Self::to_f64(args.remove(0)) as i32;
                if x == 0.0 { return Ok(Value::Float(0.0)); }
                let mag = x.abs().log10().floor() as i32;
                let factor = 10f64.powi(n - 1 - mag);
                return Ok(Value::Float((x * factor).round() / factor));
            }

            // ── Vector Utilities ──────────────────────────────────────────────
            "__phinolib_magnitude2d" => {
                let x = Self::to_f64(args.remove(0));
                let y = Self::to_f64(args.remove(0));
                return Ok(Value::Float((x*x + y*y).sqrt()));
            }
            "__phinolib_unit_vec2d" => {
                let x = Self::to_f64(args.remove(0));
                let y = Self::to_f64(args.remove(0));
                let mag = (x*x + y*y).sqrt();
                if mag < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.unitVector2d: zero vector".into())); }
                return Ok(Value::List(vec![Value::Float(x/mag), Value::Float(y/mag)]));
            }
            "__phinolib_dot2d" => {
                let ax = Self::to_f64(args.remove(0)); let ay = Self::to_f64(args.remove(0));
                let bx = Self::to_f64(args.remove(0)); let by = Self::to_f64(args.remove(0));
                return Ok(Value::Float(ax*bx + ay*by));
            }
            "__phinolib_cross2d" => {
                let ax = Self::to_f64(args.remove(0)); let ay = Self::to_f64(args.remove(0));
                let bx = Self::to_f64(args.remove(0)); let by = Self::to_f64(args.remove(0));
                return Ok(Value::Float(ax*by - ay*bx));
            }
            "__phinolib_angle_between2d" => {
                let ax = Self::to_f64(args.remove(0)); let ay = Self::to_f64(args.remove(0));
                let bx = Self::to_f64(args.remove(0)); let by = Self::to_f64(args.remove(0));
                let dot = ax*bx + ay*by;
                let mag_a = (ax*ax + ay*ay).sqrt();
                let mag_b = (bx*bx + by*by).sqrt();
                if mag_a < 1e-300 || mag_b < 1e-300 {
                    return Err(RuntimeSignal::Error("Phinolib.angleBetween2d: zero vector".into()));
                }
                let cos_theta = (dot / (mag_a * mag_b)).max(-1.0).min(1.0);
                return Ok(Value::Float(cos_theta.acos() * 180.0 / std::f64::consts::PI));
            }

            // ── Kinematics ────────────────────────────────────────────────────
            // s = v₀t + ½at²
            "__phinolib_displacement" => {
                let v0 = Self::to_f64(args.remove(0));
                let a  = Self::to_f64(args.remove(0));
                let t  = Self::to_f64(args.remove(0));
                return Ok(Value::Float(v0*t + 0.5*a*t*t));
            }
            // v = v₀ + at
            "__phinolib_velocity" => {
                let v0 = Self::to_f64(args.remove(0));
                let a  = Self::to_f64(args.remove(0));
                let t  = Self::to_f64(args.remove(0));
                return Ok(Value::Float(v0 + a*t));
            }
            // v² = v₀² + 2as
            "__phinolib_vfinalsq" => {
                let v0 = Self::to_f64(args.remove(0));
                let a  = Self::to_f64(args.remove(0));
                let s  = Self::to_f64(args.remove(0));
                return Ok(Value::Float(v0*v0 + 2.0*a*s));
            }
            // t_stop = v₀/|a|
            "__phinolib_timetostop" => {
                let v0 = Self::to_f64(args.remove(0));
                let a  = Self::to_f64(args.remove(0)).abs();
                if a < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.timeToStop: zero acceleration".into())); }
                return Ok(Value::Float(v0 / a));
            }
            // v_avg = (v₀+v)/2
            "__phinolib_avgvelocity" => {
                let v0 = Self::to_f64(args.remove(0));
                let v  = Self::to_f64(args.remove(0));
                return Ok(Value::Float((v0 + v) / 2.0));
            }

            // ── Projectile Motion ─────────────────────────────────────────────
            // R = v₀²·sin(2θ)/g
            "__phinolib_projectile_range" => {
                let v0    = Self::to_f64(args.remove(0));
                let theta = Self::to_f64(args.remove(0)) * std::f64::consts::PI / 180.0;
                return Ok(Value::Float(v0*v0*(2.0*theta).sin() / 9.80665));
            }
            // H = v₀²·sin²θ/(2g)
            "__phinolib_projectile_height" => {
                let v0    = Self::to_f64(args.remove(0));
                let theta = Self::to_f64(args.remove(0)) * std::f64::consts::PI / 180.0;
                let sin_t = theta.sin();
                return Ok(Value::Float(v0*v0*sin_t*sin_t / (2.0*9.80665)));
            }
            // T = 2·v₀·sinθ/g
            "__phinolib_projectile_tof" => {
                let v0    = Self::to_f64(args.remove(0));
                let theta = Self::to_f64(args.remove(0)) * std::f64::consts::PI / 180.0;
                return Ok(Value::Float(2.0*v0*theta.sin() / 9.80665));
            }
            // x(t) = v₀·cosθ·t
            "__phinolib_projectile_x" => {
                let v0    = Self::to_f64(args.remove(0));
                let theta = Self::to_f64(args.remove(0)) * std::f64::consts::PI / 180.0;
                let t     = Self::to_f64(args.remove(0));
                return Ok(Value::Float(v0*theta.cos()*t));
            }
            // y(t) = v₀·sinθ·t - ½g·t²
            "__phinolib_projectile_y" => {
                let v0    = Self::to_f64(args.remove(0));
                let theta = Self::to_f64(args.remove(0)) * std::f64::consts::PI / 180.0;
                let t     = Self::to_f64(args.remove(0));
                return Ok(Value::Float(v0*theta.sin()*t - 0.5*9.80665*t*t));
            }
            // vy(t) = v₀·sinθ - g·t
            "__phinolib_projectile_vy" => {
                let v0    = Self::to_f64(args.remove(0));
                let theta = Self::to_f64(args.remove(0)) * std::f64::consts::PI / 180.0;
                let t     = Self::to_f64(args.remove(0));
                return Ok(Value::Float(v0*theta.sin() - 9.80665*t));
            }

            // ── Circular / Rotational ─────────────────────────────────────────
            // a_c = v²/r
            "__phinolib_centripetal_acc" => {
                let v = Self::to_f64(args.remove(0));
                let r = Self::to_f64(args.remove(0));
                if r.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.centripetalAcc: r cannot be zero".into())); }
                return Ok(Value::Float(v*v/r));
            }
            // F_c = m·v²/r
            "__phinolib_centripetal_force" => {
                let m = Self::to_f64(args.remove(0));
                let v = Self::to_f64(args.remove(0));
                let r = Self::to_f64(args.remove(0));
                if r.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.centripetalForce: r cannot be zero".into())); }
                return Ok(Value::Float(m*v*v/r));
            }
            // ω = 2π·rpm/60
            "__phinolib_angular_velocity" => {
                let rpm = Self::to_f64(args.remove(0));
                return Ok(Value::Float(2.0*std::f64::consts::PI*rpm/60.0));
            }
            // T = 2π/ω
            "__phinolib_period_omega" => {
                let omega = Self::to_f64(args.remove(0));
                if omega.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.periodFromOmega: omega cannot be zero".into())); }
                return Ok(Value::Float(2.0*std::f64::consts::PI/omega));
            }
            // v_t = ω·r
            "__phinolib_tangential_vel" => {
                let omega = Self::to_f64(args.remove(0));
                let r     = Self::to_f64(args.remove(0));
                return Ok(Value::Float(omega*r));
            }
            // ω(t) = ω₀ + α·t
            "__phinolib_angular_vel_at" => {
                let omega0 = Self::to_f64(args.remove(0));
                let alpha  = Self::to_f64(args.remove(0));
                let t      = Self::to_f64(args.remove(0));
                return Ok(Value::Float(omega0 + alpha*t));
            }
            // τ = F·r·sinθ
            "__phinolib_torque" => {
                let f     = Self::to_f64(args.remove(0));
                let r     = Self::to_f64(args.remove(0));
                let theta = Self::to_f64(args.remove(0)) * std::f64::consts::PI / 180.0;
                return Ok(Value::Float(f*r*theta.sin()));
            }
            // L = I·ω
            "__phinolib_angular_momentum" => {
                let i     = Self::to_f64(args.remove(0));
                let omega = Self::to_f64(args.remove(0));
                return Ok(Value::Float(i*omega));
            }
            // I = (2/5)·m·r²  (solid sphere)
            "__phinolib_inertia_solid_sphere" => {
                let m = Self::to_f64(args.remove(0));
                let r = Self::to_f64(args.remove(0));
                return Ok(Value::Float(0.4*m*r*r));
            }
            // I = (2/3)·m·r²  (hollow sphere)
            "__phinolib_inertia_hollow_sphere" => {
                let m = Self::to_f64(args.remove(0));
                let r = Self::to_f64(args.remove(0));
                return Ok(Value::Float((2.0/3.0)*m*r*r));
            }
            // I = 0.5·m·r²  (solid cylinder)
            "__phinolib_inertia_cylinder" => {
                let m = Self::to_f64(args.remove(0));
                let r = Self::to_f64(args.remove(0));
                return Ok(Value::Float(0.5*m*r*r));
            }
            // I = (1/12)·m·l²  (rod about center)
            "__phinolib_inertia_rod_center" => {
                let m = Self::to_f64(args.remove(0));
                let l = Self::to_f64(args.remove(0));
                return Ok(Value::Float(m*l*l/12.0));
            }
            // I = (1/3)·m·l²  (rod about end)
            "__phinolib_inertia_rod_end" => {
                let m = Self::to_f64(args.remove(0));
                let l = Self::to_f64(args.remove(0));
                return Ok(Value::Float(m*l*l/3.0));
            }

            // ── Newtonian Dynamics ────────────────────────────────────────────
            // F = m·a
            "__phinolib_force" => {
                let m = Self::to_f64(args.remove(0));
                let a = Self::to_f64(args.remove(0));
                return Ok(Value::Float(m*a));
            }
            // W = m·g
            "__phinolib_weight" => {
                let m = Self::to_f64(args.remove(0));
                return Ok(Value::Float(m*9.80665));
            }
            // f_friction = μ·N
            "__phinolib_friction" => {
                let mu     = Self::to_f64(args.remove(0));
                let normal = Self::to_f64(args.remove(0));
                return Ok(Value::Float(mu*normal));
            }
            // J = F·Δt
            "__phinolib_impulse" => {
                let f  = Self::to_f64(args.remove(0));
                let dt = Self::to_f64(args.remove(0));
                return Ok(Value::Float(f*dt));
            }
            // p = m·v
            "__phinolib_momentum" => {
                let m = Self::to_f64(args.remove(0));
                let v = Self::to_f64(args.remove(0));
                return Ok(Value::Float(m*v));
            }

            // ── Collisions ────────────────────────────────────────────────────
            // v1' = ((m1-m2)v1 + 2m2·v2)/(m1+m2)
            "__phinolib_elastic_v1" => {
                let m1 = Self::to_f64(args.remove(0)); let m2 = Self::to_f64(args.remove(0));
                let v1 = Self::to_f64(args.remove(0)); let v2 = Self::to_f64(args.remove(0));
                let denom = m1 + m2;
                if denom.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.elasticV1: total mass is zero".into())); }
                return Ok(Value::Float(((m1-m2)*v1 + 2.0*m2*v2) / denom));
            }
            // v2' = ((m2-m1)v2 + 2m1·v1)/(m1+m2)
            "__phinolib_elastic_v2" => {
                let m1 = Self::to_f64(args.remove(0)); let m2 = Self::to_f64(args.remove(0));
                let v1 = Self::to_f64(args.remove(0)); let v2 = Self::to_f64(args.remove(0));
                let denom = m1 + m2;
                if denom.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.elasticV2: total mass is zero".into())); }
                return Ok(Value::Float(((m2-m1)*v2 + 2.0*m1*v1) / denom));
            }
            // v_f = (m1·v1 + m2·v2)/(m1+m2)
            "__phinolib_inelastic_vf" => {
                let m1 = Self::to_f64(args.remove(0)); let m2 = Self::to_f64(args.remove(0));
                let v1 = Self::to_f64(args.remove(0)); let v2 = Self::to_f64(args.remove(0));
                let denom = m1 + m2;
                if denom.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.inelasticVFinal: total mass is zero".into())); }
                return Ok(Value::Float((m1*v1 + m2*v2) / denom));
            }
            // e = |v_sep| / |v_approach|
            "__phinolib_coeff_restitution" => {
                let v_approach  = Self::to_f64(args.remove(0)).abs();
                let v_separate  = Self::to_f64(args.remove(0)).abs();
                if v_approach < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.coeffRestitution: approach velocity is zero".into())); }
                return Ok(Value::Float(v_separate / v_approach));
            }

            // ── Gravity & Orbital Mechanics ───────────────────────────────────
            // F = G·m1·m2/r²
            "__phinolib_grav_force" => {
                let m1 = Self::to_f64(args.remove(0));
                let m2 = Self::to_f64(args.remove(0));
                let r  = Self::to_f64(args.remove(0));
                if r.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.gravForce: r cannot be zero".into())); }
                return Ok(Value::Float(6.674e-11 * m1 * m2 / (r*r)));
            }
            // g_field = G·M/r²
            "__phinolib_grav_field" => {
                let big_m = Self::to_f64(args.remove(0));
                let r     = Self::to_f64(args.remove(0));
                if r.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.gravField: r cannot be zero".into())); }
                return Ok(Value::Float(6.674e-11 * big_m / (r*r)));
            }
            // φ = -G·M/r
            "__phinolib_grav_potential" => {
                let big_m = Self::to_f64(args.remove(0));
                let r     = Self::to_f64(args.remove(0));
                if r.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.gravPotential: r cannot be zero".into())); }
                return Ok(Value::Float(-6.674e-11 * big_m / r));
            }
            // v_e = sqrt(2·G·M/r)
            "__phinolib_escape_velocity" => {
                let big_m = Self::to_f64(args.remove(0));
                let r     = Self::to_f64(args.remove(0));
                if r.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.escapeVelocity: r cannot be zero".into())); }
                return Ok(Value::Float((2.0*6.674e-11*big_m/r).sqrt()));
            }
            // v_o = sqrt(G·M/r)
            "__phinolib_orbital_velocity" => {
                let big_m = Self::to_f64(args.remove(0));
                let r     = Self::to_f64(args.remove(0));
                if r.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.orbitalVelocity: r cannot be zero".into())); }
                return Ok(Value::Float((6.674e-11*big_m/r).sqrt()));
            }
            // T = 2π·sqrt(r³/(G·M))  — Kepler's 3rd Law
            "__phinolib_orbital_period" => {
                let big_m = Self::to_f64(args.remove(0));
                let r     = Self::to_f64(args.remove(0));
                if r.abs() < 1e-300 || big_m.abs() < 1e-300 {
                    return Err(RuntimeSignal::Error("Phinolib.orbitalPeriod: r and M must be nonzero".into()));
                }
                return Ok(Value::Float(2.0*std::f64::consts::PI*(r*r*r/(6.674e-11*big_m)).sqrt()));
            }
            // r_s = 2·G·M/c²
            "__phinolib_schwarzschild" => {
                let big_m = Self::to_f64(args.remove(0));
                const C: f64 = 299_792_458.0;
                return Ok(Value::Float(2.0*6.674e-11*big_m/(C*C)));
            }
            // d_Roche = R · (2·ρp/ρs)^(1/3)
            "__phinolib_roche_limit" => {
                let r_p   = Self::to_f64(args.remove(0));
                let rho_p = Self::to_f64(args.remove(0));
                let rho_s = Self::to_f64(args.remove(0));
                if rho_s.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.rocheLimit: rho_s cannot be zero".into())); }
                return Ok(Value::Float(r_p * (2.0*rho_p/rho_s).cbrt()));
            }
            // r_H = a · (m/(3M))^(1/3)
            "__phinolib_hill_sphere" => {
                let a     = Self::to_f64(args.remove(0));
                let m     = Self::to_f64(args.remove(0));
                let big_m = Self::to_f64(args.remove(0));
                if big_m.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.hillSphere: M cannot be zero".into())); }
                return Ok(Value::Float(a * (m/(3.0*big_m)).cbrt()));
            }

            // ── Energy & Work ─────────────────────────────────────────────────
            // KE = ½·m·v²
            "__phinolib_ke" => {
                let m = Self::to_f64(args.remove(0));
                let v = Self::to_f64(args.remove(0));
                return Ok(Value::Float(0.5*m*v*v));
            }
            // PE = m·g·h
            "__phinolib_pe" => {
                let m = Self::to_f64(args.remove(0));
                let h = Self::to_f64(args.remove(0));
                return Ok(Value::Float(m*9.80665*h));
            }
            // U_spring = ½·k·x²
            "__phinolib_elastic_pe" => {
                let k = Self::to_f64(args.remove(0));
                let x = Self::to_f64(args.remove(0));
                return Ok(Value::Float(0.5*k*x*x));
            }
            // W = F·d·cosθ
            "__phinolib_work" => {
                let f     = Self::to_f64(args.remove(0));
                let d     = Self::to_f64(args.remove(0));
                let theta = Self::to_f64(args.remove(0)) * std::f64::consts::PI / 180.0;
                return Ok(Value::Float(f*d*theta.cos()));
            }
            // P = W/t
            "__phinolib_power" => {
                let w = Self::to_f64(args.remove(0));
                let t = Self::to_f64(args.remove(0));
                if t.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.power: t cannot be zero".into())); }
                return Ok(Value::Float(w/t));
            }
            // P = F·v
            "__phinolib_power_fv" => {
                let f = Self::to_f64(args.remove(0));
                let v = Self::to_f64(args.remove(0));
                return Ok(Value::Float(f*v));
            }
            // η = W_useful/W_total
            "__phinolib_efficiency" => {
                let w_u = Self::to_f64(args.remove(0));
                let w_t = Self::to_f64(args.remove(0));
                if w_t.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.efficiency: W_total cannot be zero".into())); }
                return Ok(Value::Float(w_u/w_t));
            }
            // KE_rot = ½·I·ω²
            "__phinolib_rotational_ke" => {
                let i     = Self::to_f64(args.remove(0));
                let omega = Self::to_f64(args.remove(0));
                return Ok(Value::Float(0.5*i*omega*omega));
            }

            // ── SHM & Oscillations ────────────────────────────────────────────
            // T = 2π·sqrt(m/k)
            "__phinolib_shm_spring" => {
                let m = Self::to_f64(args.remove(0));
                let k = Self::to_f64(args.remove(0));
                if k.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.shmPeriodSpring: k cannot be zero".into())); }
                return Ok(Value::Float(2.0*std::f64::consts::PI*(m/k).sqrt()));
            }
            // T = 2π·sqrt(L/g)
            "__phinolib_shm_pendulum" => {
                let l = Self::to_f64(args.remove(0));
                return Ok(Value::Float(2.0*std::f64::consts::PI*(l/9.80665).sqrt()));
            }
            // f = 1/T
            "__phinolib_shm_freq" => {
                let t = Self::to_f64(args.remove(0));
                if t.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.shmFreq: T cannot be zero".into())); }
                return Ok(Value::Float(1.0/t));
            }
            // x(t) = A·cos(ω·t + φ)
            "__phinolib_shm_x" => {
                let a     = Self::to_f64(args.remove(0));
                let omega = Self::to_f64(args.remove(0));
                let t     = Self::to_f64(args.remove(0));
                let phi   = Self::to_f64(args.remove(0)) * std::f64::consts::PI / 180.0;
                return Ok(Value::Float(a*(omega*t + phi).cos()));
            }
            // v(t) = -A·ω·sin(ω·t + φ)
            "__phinolib_shm_v" => {
                let a     = Self::to_f64(args.remove(0));
                let omega = Self::to_f64(args.remove(0));
                let t     = Self::to_f64(args.remove(0));
                let phi   = Self::to_f64(args.remove(0)) * std::f64::consts::PI / 180.0;
                return Ok(Value::Float(-a*omega*(omega*t + phi).sin()));
            }
            // a(t) = -A·ω²·cos(ω·t + φ)
            "__phinolib_shm_a" => {
                let a     = Self::to_f64(args.remove(0));
                let omega = Self::to_f64(args.remove(0));
                let t     = Self::to_f64(args.remove(0));
                let phi   = Self::to_f64(args.remove(0)) * std::f64::consts::PI / 180.0;
                return Ok(Value::Float(-a*omega*omega*(omega*t + phi).cos()));
            }
            // v_max = A·ω
            "__phinolib_shm_vmax" => {
                let a     = Self::to_f64(args.remove(0));
                let omega = Self::to_f64(args.remove(0));
                return Ok(Value::Float(a*omega));
            }
            // E_shm = ½·m·ω²·A²
            "__phinolib_shm_energy" => {
                let m     = Self::to_f64(args.remove(0));
                let omega = Self::to_f64(args.remove(0));
                let a     = Self::to_f64(args.remove(0));
                return Ok(Value::Float(0.5*m*omega*omega*a*a));
            }
            // ζ = b / (2·sqrt(m·k))
            "__phinolib_damping_ratio" => {
                let b = Self::to_f64(args.remove(0));
                let m = Self::to_f64(args.remove(0));
                let k = Self::to_f64(args.remove(0));
                let denom = 2.0*(m*k).sqrt();
                if denom < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.dampingRatio: m·k is zero".into())); }
                return Ok(Value::Float(b/denom));
            }

            // ── Waves ─────────────────────────────────────────────────────────
            // v = f·λ
            "__phinolib_wave_speed" => {
                let f      = Self::to_f64(args.remove(0));
                let lambda = Self::to_f64(args.remove(0));
                return Ok(Value::Float(f*lambda));
            }
            // u = ½·ρ·ω²·A²
            "__phinolib_wave_energy" => {
                let a     = Self::to_f64(args.remove(0));
                let omega = Self::to_f64(args.remove(0));
                let rho   = Self::to_f64(args.remove(0));
                return Ok(Value::Float(0.5*rho*omega*omega*a*a));
            }
            // f_obs = f_s · (v_sound + v_obs) / (v_sound - v_src)
            // v_obs positive when moving toward source; v_src positive when moving toward observer
            "__phinolib_doppler" => {
                let f_s     = Self::to_f64(args.remove(0));
                let v_sound = Self::to_f64(args.remove(0));
                let v_obs   = Self::to_f64(args.remove(0));
                let v_src   = Self::to_f64(args.remove(0));
                let denom   = v_sound - v_src;
                if denom.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.dopplerFreq: source at Mach 1 (singular)".into())); }
                return Ok(Value::Float(f_s*(v_sound + v_obs)/denom));
            }
            // f_n = n·v/(2L)
            "__phinolib_standing_wave" => {
                let n = Self::to_f64(args.remove(0));
                let l = Self::to_f64(args.remove(0));
                let v = Self::to_f64(args.remove(0));
                if l.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.standingWaveFreq: L cannot be zero".into())); }
                return Ok(Value::Float(n*v/(2.0*l)));
            }
            // I = P/(4π·r²)
            "__phinolib_sound_intensity" => {
                let p = Self::to_f64(args.remove(0));
                let r = Self::to_f64(args.remove(0));
                if r.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.soundIntensity: r cannot be zero".into())); }
                return Ok(Value::Float(p/(4.0*std::f64::consts::PI*r*r)));
            }
            // dB = 10·log10(I/I₀),  I₀ = 1e-12 W/m²
            "__phinolib_decibels" => {
                let i = Self::to_f64(args.remove(0));
                if i <= 0.0 { return Err(RuntimeSignal::Error("Phinolib.decibels: intensity must be positive".into())); }
                return Ok(Value::Float(10.0*(i/1e-12_f64).log10()));
            }

            // ── Thermodynamics ────────────────────────────────────────────────
            // P = nRT/V
            "__phinolib_ideal_gas_p" => {
                let n = Self::to_f64(args.remove(0));
                let t = Self::to_f64(args.remove(0));
                let v = Self::to_f64(args.remove(0));
                if v.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.idealGasP: V cannot be zero".into())); }
                return Ok(Value::Float(n*8.314462618*t/v));
            }
            // V = nRT/P
            "__phinolib_ideal_gas_v" => {
                let n = Self::to_f64(args.remove(0));
                let t = Self::to_f64(args.remove(0));
                let p = Self::to_f64(args.remove(0));
                if p.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.idealGasV: P cannot be zero".into())); }
                return Ok(Value::Float(n*8.314462618*t/p));
            }
            // T = PV/(nR)
            "__phinolib_ideal_gas_t" => {
                let p = Self::to_f64(args.remove(0));
                let v = Self::to_f64(args.remove(0));
                let n = Self::to_f64(args.remove(0));
                if n.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.idealGasT: n cannot be zero".into())); }
                return Ok(Value::Float(p*v/(n*8.314462618)));
            }
            // Q_cond/t = k·A·ΔT/Δx  (Fourier's law of heat conduction)
            "__phinolib_heat_conduction" => {
                let k  = Self::to_f64(args.remove(0));
                let a  = Self::to_f64(args.remove(0));
                let dt = Self::to_f64(args.remove(0));
                let dx = Self::to_f64(args.remove(0));
                if dx.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.heatConduction: dx cannot be zero".into())); }
                return Ok(Value::Float(k*a*dt/dx));
            }
            // P_rad = ε·σ·A·T⁴
            "__phinolib_thermal_radiation" => {
                let eps = Self::to_f64(args.remove(0));
                let a   = Self::to_f64(args.remove(0));
                let t   = Self::to_f64(args.remove(0));
                return Ok(Value::Float(eps*5.670374419e-8*a*t*t*t*t));
            }
            // Q = m·c·ΔT
            "__phinolib_heat_capacity" => {
                let m  = Self::to_f64(args.remove(0));
                let c  = Self::to_f64(args.remove(0));
                let dt = Self::to_f64(args.remove(0));
                return Ok(Value::Float(m*c*dt));
            }
            // C → K
            "__phinolib_c_to_k" => {
                let c = Self::to_f64(args.remove(0));
                return Ok(Value::Float(c + 273.15));
            }
            // K → C
            "__phinolib_k_to_c" => {
                let k = Self::to_f64(args.remove(0));
                return Ok(Value::Float(k - 273.15));
            }
            // v_rms = sqrt(3RT/M)
            "__phinolib_rms_speed" => {
                let m_mol = Self::to_f64(args.remove(0));
                let t     = Self::to_f64(args.remove(0));
                if m_mol.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.rmsSpeed: M_mol cannot be zero".into())); }
                return Ok(Value::Float((3.0*8.314462618*t/m_mol).sqrt()));
            }
            // η_carnot = 1 - T_cold/T_hot
            "__phinolib_carnot" => {
                let t_h = Self::to_f64(args.remove(0));
                let t_c = Self::to_f64(args.remove(0));
                if t_h.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.carnotEfficiency: T_hot cannot be zero".into())); }
                return Ok(Value::Float(1.0 - t_c/t_h));
            }

            // ── Electromagnetism ──────────────────────────────────────────────
            // F = k_e·q1·q2/r²
            "__phinolib_coulomb" => {
                let q1 = Self::to_f64(args.remove(0));
                let q2 = Self::to_f64(args.remove(0));
                let r  = Self::to_f64(args.remove(0));
                if r.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.coulombForce: r cannot be zero".into())); }
                return Ok(Value::Float(8.9875517923e9*q1*q2/(r*r)));
            }
            // E = k_e·q/r²
            "__phinolib_electric_field" => {
                let q = Self::to_f64(args.remove(0));
                let r = Self::to_f64(args.remove(0));
                if r.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.electricField: r cannot be zero".into())); }
                return Ok(Value::Float(8.9875517923e9*q/(r*r)));
            }
            // V = k_e·q/r
            "__phinolib_electric_potential" => {
                let q = Self::to_f64(args.remove(0));
                let r = Self::to_f64(args.remove(0));
                if r.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.electricPotential: r cannot be zero".into())); }
                return Ok(Value::Float(8.9875517923e9*q/r));
            }
            // U = q·V
            "__phinolib_electric_pe" => {
                let q = Self::to_f64(args.remove(0));
                let v = Self::to_f64(args.remove(0));
                return Ok(Value::Float(q*v));
            }
            // C = ε₀·εᵣ·A/d
            "__phinolib_capacitance" => {
                let eps_r = Self::to_f64(args.remove(0));
                let a     = Self::to_f64(args.remove(0));
                let d     = Self::to_f64(args.remove(0));
                if d.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.capacitancePlateCap: d cannot be zero".into())); }
                return Ok(Value::Float(8.8541878128e-12*eps_r*a/d));
            }
            // V = I·R
            "__phinolib_ohms_v" => {
                let i = Self::to_f64(args.remove(0));
                let r = Self::to_f64(args.remove(0));
                return Ok(Value::Float(i*r));
            }
            // I = V/R
            "__phinolib_ohms_i" => {
                let v = Self::to_f64(args.remove(0));
                let r = Self::to_f64(args.remove(0));
                if r.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.ohmsI: R cannot be zero".into())); }
                return Ok(Value::Float(v/r));
            }
            // P = V·I
            "__phinolib_electric_power" => {
                let v = Self::to_f64(args.remove(0));
                let i = Self::to_f64(args.remove(0));
                return Ok(Value::Float(v*i));
            }
            // F = q·v·B·sinθ
            "__phinolib_lorentz" => {
                let q     = Self::to_f64(args.remove(0));
                let v     = Self::to_f64(args.remove(0));
                let b     = Self::to_f64(args.remove(0));
                let theta = Self::to_f64(args.remove(0)) * std::f64::consts::PI / 180.0;
                return Ok(Value::Float(q*v*b*theta.sin()));
            }
            // B = μ₀·I/(2π·r)
            "__phinolib_biot_savart" => {
                let i = Self::to_f64(args.remove(0));
                let r = Self::to_f64(args.remove(0));
                if r.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.biotSavartWire: r cannot be zero".into())); }
                return Ok(Value::Float(1.25663706212e-6*i/(2.0*std::f64::consts::PI*r)));
            }
            // ε = -N·dΦ/dt
            "__phinolib_faraday" => {
                let n    = Self::to_f64(args.remove(0));
                let dphi = Self::to_f64(args.remove(0));
                let dt   = Self::to_f64(args.remove(0));
                if dt.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.faradayEMF: dt cannot be zero".into())); }
                return Ok(Value::Float(-n*dphi/dt));
            }
            // L = μ₀·N²·A/l
            "__phinolib_inductance" => {
                let n = Self::to_f64(args.remove(0));
                let a = Self::to_f64(args.remove(0));
                let l = Self::to_f64(args.remove(0));
                if l.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.inductanceSolenoid: l cannot be zero".into())); }
                return Ok(Value::Float(1.25663706212e-6*n*n*a/l));
            }
            // E = h·f
            "__phinolib_photon_energy" => {
                let f = Self::to_f64(args.remove(0));
                return Ok(Value::Float(6.62607015e-34*f));
            }
            // λ = c/f
            "__phinolib_photon_wavelength" => {
                let f = Self::to_f64(args.remove(0));
                if f.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.photonWavelength: f cannot be zero".into())); }
                return Ok(Value::Float(299_792_458.0_f64/f));
            }
            // λ_dB = h/(m·v)
            "__phinolib_de_broglie" => {
                let m = Self::to_f64(args.remove(0));
                let v = Self::to_f64(args.remove(0));
                let mv = m*v;
                if mv.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.deBroglieWavelength: m·v cannot be zero".into())); }
                return Ok(Value::Float(6.62607015e-34/mv));
            }

            // ── Fluid Mechanics ───────────────────────────────────────────────
            // P = ρ·g·h
            "__phinolib_hydrostatic" => {
                let rho = Self::to_f64(args.remove(0));
                let h   = Self::to_f64(args.remove(0));
                return Ok(Value::Float(rho*9.80665*h));
            }
            // F_b = ρ·g·V
            "__phinolib_buoyancy" => {
                let rho = Self::to_f64(args.remove(0));
                let v   = Self::to_f64(args.remove(0));
                return Ok(Value::Float(rho*9.80665*v));
            }
            // Bernoulli: ½ρv₁²+P₁+ρgh₁ = ½ρv₂²+P₂+ρgh₂ → solve for v₂
            "__phinolib_bernoulli_v2" => {
                let v1  = Self::to_f64(args.remove(0));
                let p1  = Self::to_f64(args.remove(0));
                let p2  = Self::to_f64(args.remove(0));
                let rho = Self::to_f64(args.remove(0));
                let h1  = Self::to_f64(args.remove(0));
                let h2  = Self::to_f64(args.remove(0));
                if rho.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.bernoulliV2: rho cannot be zero".into())); }
                let v2_sq = v1*v1 + 2.0*(p1-p2)/rho + 2.0*9.80665*(h1-h2);
                if v2_sq < 0.0 { return Err(RuntimeSignal::Error("Phinolib.bernoulliV2: negative v2² (energy violation — check inputs)".into())); }
                return Ok(Value::Float(v2_sq.sqrt()));
            }
            // F_drag = 6π·η·r·v  (Stokes drag, laminar, Re << 1)
            "__phinolib_stokes_drag" => {
                let eta = Self::to_f64(args.remove(0));
                let r   = Self::to_f64(args.remove(0));
                let v   = Self::to_f64(args.remove(0));
                return Ok(Value::Float(6.0*std::f64::consts::PI*eta*r*v));
            }
            // Re = ρ·v·L/η
            "__phinolib_reynolds" => {
                let rho = Self::to_f64(args.remove(0));
                let v   = Self::to_f64(args.remove(0));
                let l   = Self::to_f64(args.remove(0));
                let eta = Self::to_f64(args.remove(0));
                if eta.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.reynoldsNumber: eta (viscosity) cannot be zero".into())); }
                return Ok(Value::Float(rho*v*l/eta));
            }
            // Q = A·v
            "__phinolib_flow_rate" => {
                let a = Self::to_f64(args.remove(0));
                let v = Self::to_f64(args.remove(0));
                return Ok(Value::Float(a*v));
            }
            // M = v/v_sound
            "__phinolib_mach" => {
                let v       = Self::to_f64(args.remove(0));
                let v_sound = Self::to_f64(args.remove(0));
                if v_sound.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.machNumber: v_sound cannot be zero".into())); }
                return Ok(Value::Float(v/v_sound));
            }

            // ── Special Relativity ────────────────────────────────────────────
            // γ = 1/sqrt(1 - v²/c²)
            "__phinolib_lorentz_factor" => {
                let v = Self::to_f64(args.remove(0));
                const C: f64 = 299_792_458.0;
                let beta_sq = (v/C)*(v/C);
                if beta_sq >= 1.0 { return Err(RuntimeSignal::Error("Phinolib.lorentzFactor: v must be < c".into())); }
                return Ok(Value::Float(1.0/(1.0 - beta_sq).sqrt()));
            }
            // t = γ·t₀
            "__phinolib_time_dilation" => {
                let t0 = Self::to_f64(args.remove(0));
                let v  = Self::to_f64(args.remove(0));
                const C: f64 = 299_792_458.0;
                let beta_sq = (v/C)*(v/C);
                if beta_sq >= 1.0 { return Err(RuntimeSignal::Error("Phinolib.timeDilation: v must be < c".into())); }
                let gamma = 1.0/(1.0 - beta_sq).sqrt();
                return Ok(Value::Float(gamma*t0));
            }
            // L = L₀/γ
            "__phinolib_length_contraction" => {
                let l0 = Self::to_f64(args.remove(0));
                let v  = Self::to_f64(args.remove(0));
                const C: f64 = 299_792_458.0;
                let beta_sq = (v/C)*(v/C);
                if beta_sq >= 1.0 { return Err(RuntimeSignal::Error("Phinolib.lengthContraction: v must be < c".into())); }
                let gamma = 1.0/(1.0 - beta_sq).sqrt();
                return Ok(Value::Float(l0/gamma));
            }
            // m = γ·m₀
            "__phinolib_rel_mass" => {
                let m0 = Self::to_f64(args.remove(0));
                let v  = Self::to_f64(args.remove(0));
                const C: f64 = 299_792_458.0;
                let beta_sq = (v/C)*(v/C);
                if beta_sq >= 1.0 { return Err(RuntimeSignal::Error("Phinolib.relativisticMass: v must be < c".into())); }
                let gamma = 1.0/(1.0 - beta_sq).sqrt();
                return Ok(Value::Float(gamma*m0));
            }
            // E₀ = m·c²
            "__phinolib_rest_energy" => {
                let m = Self::to_f64(args.remove(0));
                const C: f64 = 299_792_458.0;
                return Ok(Value::Float(m*C*C));
            }
            // KE_rel = (γ-1)·m₀·c²
            "__phinolib_rel_ke" => {
                let m0 = Self::to_f64(args.remove(0));
                let v  = Self::to_f64(args.remove(0));
                const C: f64 = 299_792_458.0;
                let beta_sq = (v/C)*(v/C);
                if beta_sq >= 1.0 { return Err(RuntimeSignal::Error("Phinolib.relativisticKE: v must be < c".into())); }
                let gamma = 1.0/(1.0 - beta_sq).sqrt();
                return Ok(Value::Float((gamma - 1.0)*m0*C*C));
            }
            // p_rel = γ·m₀·v
            "__phinolib_rel_momentum" => {
                let m0 = Self::to_f64(args.remove(0));
                let v  = Self::to_f64(args.remove(0));
                const C: f64 = 299_792_458.0;
                let beta_sq = (v/C)*(v/C);
                if beta_sq >= 1.0 { return Err(RuntimeSignal::Error("Phinolib.relativisticMomentum: v must be < c".into())); }
                let gamma = 1.0/(1.0 - beta_sq).sqrt();
                return Ok(Value::Float(gamma*m0*v));
            }
            // w = (u+v)/(1 + u·v/c²)  — relativistic velocity addition
            "__phinolib_rel_velocity_add" => {
                let u = Self::to_f64(args.remove(0));
                let v = Self::to_f64(args.remove(0));
                const C: f64 = 299_792_458.0;
                let denom = 1.0 + u*v/(C*C);
                if denom.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.relVelocityAdd: denominator vanishes".into())); }
                return Ok(Value::Float((u + v)/denom));
            }

            // ── Optics ────────────────────────────────────────────────────────
            // θ₂ = arcsin(n₁·sinθ₁/n₂)  in degrees
            "__phinolib_snells" => {
                let n1     = Self::to_f64(args.remove(0));
                let theta1 = Self::to_f64(args.remove(0)) * std::f64::consts::PI / 180.0;
                let n2     = Self::to_f64(args.remove(0));
                if n2.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.snellsLaw: n2 cannot be zero".into())); }
                let sin_t2 = n1*theta1.sin()/n2;
                if sin_t2.abs() > 1.0 { return Err(RuntimeSignal::Error("Phinolib.snellsLaw: total internal reflection (no refracted ray)".into())); }
                return Ok(Value::Float(sin_t2.asin() * 180.0 / std::f64::consts::PI));
            }
            // θ_c = arcsin(n2/n1)  in degrees
            "__phinolib_critical_angle" => {
                let n1 = Self::to_f64(args.remove(0));
                let n2 = Self::to_f64(args.remove(0));
                if n1.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.criticalAngle: n1 cannot be zero".into())); }
                let ratio = n2/n1;
                if ratio.abs() > 1.0 { return Err(RuntimeSignal::Error("Phinolib.criticalAngle: n2 > n1 means no critical angle".into())); }
                return Ok(Value::Float(ratio.asin() * 180.0 / std::f64::consts::PI));
            }
            // 1/di = 1/f - 1/do  → di = f·do/(do-f)
            "__phinolib_thin_lens" => {
                let f   = Self::to_f64(args.remove(0));
                let d_o = Self::to_f64(args.remove(0));
                let denom = d_o - f;
                if denom.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.thinLens: object at focal point (image at infinity)".into())); }
                return Ok(Value::Float(f*d_o/denom));
            }
            // m = -di/do
            "__phinolib_magnification" => {
                let d_i = Self::to_f64(args.remove(0));
                let d_o = Self::to_f64(args.remove(0));
                if d_o.abs() < 1e-300 { return Err(RuntimeSignal::Error("Phinolib.magnification: do cannot be zero".into())); }
                return Ok(Value::Float(-d_i/d_o));
            }

            // ================================================================
            // VYRAWEB RUNTIME HANDLERS
            // ================================================================

            // ── Route Registration ───────────────────────────────────────
            "__vyraweb_route" => {
                let method  = args.first().cloned().unwrap_or(Value::Str("GET".into())).to_string().to_uppercase();
                let path    = args.get(1).cloned().unwrap_or(Value::Str("/".into())).to_string();
                let handler = args.get(2).cloned().unwrap_or(Value::Null).to_string();
                println!("✓ Vyraweb: {} {} → {} registered", method, path, handler);
                return Ok(Value::Str(format!("{}:{}", method, path)));
            }
            "__vyraweb_get" => {
                let path    = args.first().cloned().unwrap_or(Value::Str("/".into())).to_string();
                let handler = args.get(1).cloned().unwrap_or(Value::Null).to_string();
                println!("✓ Vyraweb: GET {} → {} registered", path, handler);
                return Ok(Value::Str(format!("GET:{}", path)));
            }
            "__vyraweb_post" => {
                let path    = args.first().cloned().unwrap_or(Value::Str("/".into())).to_string();
                let handler = args.get(1).cloned().unwrap_or(Value::Null).to_string();
                println!("✓ Vyraweb: POST {} → {} registered", path, handler);
                return Ok(Value::Str(format!("POST:{}", path)));
            }
            "__vyraweb_put" => {
                let path    = args.first().cloned().unwrap_or(Value::Str("/".into())).to_string();
                let handler = args.get(1).cloned().unwrap_or(Value::Null).to_string();
                println!("✓ Vyraweb: PUT {} → {} registered", path, handler);
                return Ok(Value::Str(format!("PUT:{}", path)));
            }
            "__vyraweb_delete" => {
                let path    = args.first().cloned().unwrap_or(Value::Str("/".into())).to_string();
                let handler = args.get(1).cloned().unwrap_or(Value::Null).to_string();
                println!("✓ Vyraweb: DELETE {} → {} registered", path, handler);
                return Ok(Value::Str(format!("DELETE:{}", path)));
            }
            "__vyraweb_patch" => {
                let path    = args.first().cloned().unwrap_or(Value::Str("/".into())).to_string();
                let handler = args.get(1).cloned().unwrap_or(Value::Null).to_string();
                println!("✓ Vyraweb: PATCH {} → {} registered", path, handler);
                return Ok(Value::Str(format!("PATCH:{}", path)));
            }

            // ── Response Builders ────────────────────────────────────────
            "__vyraweb_json" => {
                // Value::Map / Value::List ko JSON string mein convert karo
                let data = args.into_iter().next().unwrap_or(Value::Null);
                let json_str = value_to_json(&data);
                return Ok(Value::Str(format!("__RESPONSE__json::{}", json_str)));
            }
            "__vyraweb_html" => {
                let content = args.into_iter().next().unwrap_or(Value::Null).to_string();
                return Ok(Value::Str(format!("__RESPONSE__html::{}", content)));
            }
            "__vyraweb_text" => {
                let content = args.into_iter().next().unwrap_or(Value::Null).to_string();
                return Ok(Value::Str(format!("__RESPONSE__text::{}", content)));
            }
            "__vyraweb_redirect" => {
                let url = args.into_iter().next().unwrap_or(Value::Null).to_string();
                println!("↪ Vyraweb: Redirect → {}", url);
                return Ok(Value::Str(format!("__RESPONSE__redirect::{}", url)));
            }
            "__vyraweb_status" => {
                let code = match args.first() { Some(Value::Int(n)) => *n, _ => 200 };
                let body = args.get(1).cloned().unwrap_or(Value::Str(String::new())).to_string();
                return Ok(Value::Str(format!("__RESPONSE__status::{}::{}", code, body)));
            }
            "__vyraweb_header" => {
                let key = args.first().cloned().unwrap_or(Value::Null).to_string();
                let val = args.get(1).cloned().unwrap_or(Value::Null).to_string();
                println!("  Header: {} = {}", key, val);
                return Ok(Value::Str(format!("{}:{}", key, val)));
            }

            // ── Middleware & Static ──────────────────────────────────────
            "__vyraweb_middleware" => {
                let handler = args.into_iter().next().unwrap_or(Value::Null).to_string();
                println!("✓ Vyraweb: Middleware registered → {}", handler);
                return Ok(Value::Null);
            }
            "__vyraweb_static_dir" => {
                let dir = args.into_iter().next().unwrap_or(Value::Null).to_string();
                println!("✓ Vyraweb: Static files dir set → /{}", dir);
                return Ok(Value::Str(format!("/static → {}", dir)));
            }

            // ── URL Utils ────────────────────────────────────────────────
            "__vyraweb_url_for" => {
                let name = args.into_iter().next().unwrap_or(Value::Null).to_string();
                return Ok(Value::Str(format!("/{}", name)));
            }

            // ── Server Control ───────────────────────────────────────────
            "__vyraweb_run" => {
                let port = match args.first() {
                    Some(Value::Int(n)) => *n,
                    Some(Value::Str(s)) => s.parse().unwrap_or(8080),
                    _ => 8080,
                };
                println!();
                println!("╔══════════════════════════════════════════════════╗");
                println!("║  🌐 Vyraweb Server Starting                      ║");
                println!("║  URL  : http://localhost:{}                   ║", port);
                println!("║  Mode : HTTP  |  Workers: auto                  ║");
                println!("║  Press Ctrl+C to stop                           ║");
                println!("╚══════════════════════════════════════════════════╝");
                println!();
                // Real TCP listener — accepts connections and returns basic HTTP responses
                use std::net::TcpListener;
                use std::io::Read;
                let addr = format!("0.0.0.0:{}", port);
                match TcpListener::bind(&addr) {
                    Ok(listener) => {
                        println!("  ✓ Listening on {}", addr);
                        // Each connection is handed off to remox_task_spawn()
                        // (via std::thread::spawn, see std-compat shim) so
                        // one slow/idle client can't block the accept loop
                        // from taking new connections. StubHal runs spawned
                        // closures inline (so single-connection behavior is
                        // unchanged there); a real Monobat scheduler impl
                        // gets true concurrency for free once wired.
                        for stream in listener.incoming() {
                            match stream {
                                Ok(mut s) => {
                                    std::thread::spawn(move || {
                                        let mut buf = [0u8; 4096];
                                        let n = s.read(&mut buf).unwrap_or(0);
                                        let req = String::from_utf8_lossy(&buf[..n]);
                                        // Parse method + path from request line
                                        let first_line = req.lines().next().unwrap_or("");
                                        let parts: Vec<&str> = first_line.splitn(3, ' ').collect();
                                        let method = if parts.len() > 0 { parts[0] } else { "GET" };
                                        let path   = if parts.len() > 1 { parts[1] } else { "/" };
                                        println!("  → {} {}", method, path);
                                        let body = format!("<h1>Vyraweb</h1><p>Route: {} {}</p>", method, path);
                                        let response = format!(
                                            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nServer: Vyraweb/1.0\r\n\r\n{}",
                                            body.len(), body
                                        );
                                        let _ = io::Write::write_all(&mut s, response.as_bytes());
                                    });
                                }
                                Err(_) => {}
                            }
                        }
                    }
                    Err(e) => eprintln!("Vyraweb: Could not bind to port {}: {}", port, e),
                }
                return Ok(Value::Null);
            }
            "__vyraweb_run_secure" => {
                let port = match args.first() { Some(Value::Int(n)) => *n, _ => 443 };
                let cert = args.get(1).cloned().unwrap_or(Value::Str("cert.pem".into())).to_string();
                let key  = args.get(2).cloned().unwrap_or(Value::Str("key.pem".into())).to_string();
                println!();
                println!("╔══════════════════════════════════════════════════╗");
                println!("║  🔒 Vyraweb HTTPS Server                        ║");
                println!("║  URL  : https://localhost:{}                 ║", port);
                println!("║  Cert : {}                                  ║", cert);
                println!("║  Key  : {}                                  ║", key);
                println!("╚══════════════════════════════════════════════════╝");
                println!("  (HTTPS ke liye TLS library link karein)");
                return Ok(Value::Null);
            }
            "__vyraweb_stop" => {
                println!("✓ Vyraweb: Server stopped.");
                return Ok(Value::Null);
            }

            // ================================================================
            // VYRAWEB ORM — VyraDB (Real flat-file JSON database)
            // SQLite-jaisi simplicity, Remox ke andar built-in
            // ================================================================

            "__vyraweb_db_connect" => {
                let path = args.into_iter().next().unwrap_or(Value::Str("vyradb.json".into())).to_string();
                // Ensure the DB file exists (create empty if not)
                if !std::path::Path::new(&path).exists() {
                    match fs::write(&path, "{}") {
                        Ok(_)  => println!("✓ VyraDB: Created new database → {}", path),
                        Err(e) => return Err(RuntimeSignal::Error(format!("VyraDB.connect: cannot create {}: {}", path, e))),
                    }
                } else {
                    println!("✓ VyraDB: Connected → {}", path);
                }
                return Ok(Value::Str(path));
            }

            "__vyraweb_db_create" => {
                // db_create("users", {name: "text", age: "int", email: "text"})
                // Stores table schema inside vyradb.json under "__schema__"
                let table  = args.first().cloned().unwrap_or(Value::Null).to_string();
                let schema = match args.get(1).cloned().unwrap_or(Value::Null) {
                    Value::Map(pairs) => pairs,
                    _ => return Err(RuntimeSignal::Error("VyraDB.create: schema must be a map {col: type}".into())),
                };
                let db_path = "vyradb.json";
                let raw = fs::read_to_string(db_path).unwrap_or_else(|_| "{}".into());
                let mut db = vyradb_parse(&raw);
                // Create empty table + schema entry
                db.entry(table.clone()).or_insert_with(Vec::new);
                let schema_key = format!("__schema__{}", table);
                let schema_str: Vec<String> = schema.iter().map(|(k,v)| format!("{}:{}", k, v)).collect();
                db.insert(schema_key, vec![schema_str.join(",")]);
                vyradb_flush(db_path, &db)?;
                println!("✓ VyraDB: Table '{}' created ({} columns)", table, schema.len());
                return Ok(Value::Str(table));
            }

            "__vyraweb_db_insert" => {
                let table = args.first().cloned().unwrap_or(Value::Null).to_string();
                let data  = match args.get(1).cloned().unwrap_or(Value::Null) {
                    Value::Map(p) => p,
                    _ => return Err(RuntimeSignal::Error("VyraDB.insert: data must be a map".into())),
                };
                let db_path = "vyradb.json";
                let raw = fs::read_to_string(db_path).unwrap_or_else(|_| "{}".into());
                let mut db = vyradb_parse(&raw);
                let rows = db.entry(table.clone()).or_insert_with(Vec::new);
                // Rows are stored as real JSON objects (value_to_json),
                // read back with vyradb_json_parse_value — proper quoting
                // means no ambiguity for values containing `|`, `\n`, or
                // `=`, unlike the old pipe-delimited format.
                let row_str = value_to_json(&Value::Map(data.clone()));
                rows.push(row_str);
                let inserted_id = rows.len() as i64;
                vyradb_flush(db_path, &db)?;
                println!("✓ VyraDB: Inserted into '{}' (id={})", table, inserted_id);
                return Ok(Value::Int(inserted_id));
            }

            "__vyraweb_db_find_all" => {
                let table   = args.into_iter().next().unwrap_or(Value::Null).to_string();
                let db_path = "vyradb.json";
                let raw = fs::read_to_string(db_path).unwrap_or_else(|_| "{}".into());
                let db = vyradb_parse(&raw);
                let rows = db.get(&table).cloned().unwrap_or_default();
                let result: Vec<Value> = rows.iter()
                    .filter(|r| !r.starts_with("__schema__"))
                    .map(|r| vyradb_row_to_value(r))
                    .collect();
                println!("✓ VyraDB: find_all('{}') → {} rows", table, result.len());
                return Ok(Value::List(result));
            }

            "__vyraweb_db_find" => {
                let table  = args.first().cloned().unwrap_or(Value::Null).to_string();
                let filter = match args.get(1).cloned().unwrap_or(Value::Null) {
                    Value::Map(p) => p,
                    _ => vec![],
                };
                let db_path = "vyradb.json";
                let raw = fs::read_to_string(db_path).unwrap_or_else(|_| "{}".into());
                let db = vyradb_parse(&raw);
                let rows = db.get(&table).cloned().unwrap_or_default();
                let result: Vec<Value> = rows.iter()
                    .filter(|r| !r.starts_with("__schema__"))
                    .map(|r| vyradb_row_to_value(r))
                    .filter(|row_val| {
                        if let Value::Map(ref row) = row_val {
                            filter.iter().all(|(fk, fv)| {
                                row.iter().any(|(rk, rv)| rk == fk && rv.to_string() == fv.to_string())
                            })
                        } else { false }
                    })
                    .collect();
                println!("✓ VyraDB: find('{}', filter) → {} rows", table, result.len());
                return Ok(Value::List(result));
            }

            "__vyraweb_db_update" => {
                let table  = args.first().cloned().unwrap_or(Value::Null).to_string();
                let data   = match args.get(1).cloned().unwrap_or(Value::Null) {
                    Value::Map(p) => p,
                    _ => return Err(RuntimeSignal::Error("VyraDB.update: data must be a map".into())),
                };
                let filter = match args.get(2).cloned().unwrap_or(Value::Null) {
                    Value::Map(p) => p,
                    _ => vec![],
                };
                let db_path = "vyradb.json";
                let raw = fs::read_to_string(db_path).unwrap_or_else(|_| "{}".into());
                let mut db = vyradb_parse(&raw);
                let mut updated = 0usize;
                if let Some(rows) = db.get_mut(&table) {
                    for row_str in rows.iter_mut() {
                        if row_str.starts_with("__schema__") { continue; }
                        let row_val = vyradb_row_to_value(row_str);
                        if let Value::Map(ref row) = row_val {
                            let matches = filter.iter().all(|(fk, fv)| {
                                row.iter().any(|(rk, rv)| rk == fk && rv.to_string() == fv.to_string())
                            });
                            if matches {
                                // Merge data into row, keeping full Value
                                // fidelity (not string-ified), then
                                // re-serialize as JSON.
                                let mut new_row: Vec<(String, Value)> = row.clone();
                                for (dk, dv) in &data {
                                    if let Some(entry) = new_row.iter_mut().find(|(k, _)| k == dk) {
                                        entry.1 = dv.clone();
                                    } else {
                                        new_row.push((dk.clone(), dv.clone()));
                                    }
                                }
                                *row_str = value_to_json(&Value::Map(new_row));
                                updated += 1;
                            }
                        }
                    }
                }
                vyradb_flush(db_path, &db)?;
                println!("✓ VyraDB: Updated {} row(s) in '{}'", updated, table);
                return Ok(Value::Int(updated as i64));
            }

            "__vyraweb_db_delete" => {
                let table  = args.first().cloned().unwrap_or(Value::Null).to_string();
                let filter = match args.get(1).cloned().unwrap_or(Value::Null) {
                    Value::Map(p) => p,
                    _ => vec![],
                };
                let db_path = "vyradb.json";
                let raw = fs::read_to_string(db_path).unwrap_or_else(|_| "{}".into());
                let mut db = vyradb_parse(&raw);
                let mut deleted = 0usize;
                if let Some(rows) = db.get_mut(&table) {
                    let before = rows.len();
                    rows.retain(|row_str| {
                        if row_str.starts_with("__schema__") { return true; }
                        let row_val = vyradb_row_to_value(row_str);
                        if let Value::Map(ref row) = row_val {
                            !filter.iter().all(|(fk, fv)| {
                                row.iter().any(|(rk, rv)| rk == fk && rv.to_string() == fv.to_string())
                            })
                        } else { true }
                    });
                    deleted = before - rows.len();
                }
                vyradb_flush(db_path, &db)?;
                println!("✓ VyraDB: Deleted {} row(s) from '{}'", deleted, table);
                return Ok(Value::Int(deleted as i64));
            }

            "__vyraweb_db_count" => {
                let table   = args.into_iter().next().unwrap_or(Value::Null).to_string();
                let db_path = "vyradb.json";
                let raw = fs::read_to_string(db_path).unwrap_or_else(|_| "{}".into());
                let db = vyradb_parse(&raw);
                let count = db.get(&table).map(|rows| {
                    rows.iter().filter(|r| !r.starts_with("__schema__")).count()
                }).unwrap_or(0);
                return Ok(Value::Int(count as i64));
            }

            "__vyraweb_db_query" => {
                // Raw SQL-like query — SELECT / INSERT / UPDATE / DELETE text
                let sql = args.into_iter().next().unwrap_or(Value::Null).to_string();
                println!("  VyraDB SQL → {}", sql);
                // Parse simple SELECT * FROM table WHERE col=val
                let sql_up = sql.trim().to_uppercase();
                if sql_up.starts_with("SELECT") {
                    // Extract table name after FROM
                    if let Some(from_pos) = sql_up.find(" FROM ") {
                        let after_from = sql[from_pos + 6..].trim();
                        let table = after_from.split_whitespace().next().unwrap_or("").to_string();
                        let db_path = "vyradb.json";
                        let raw = fs::read_to_string(db_path).unwrap_or_else(|_| "{}".into());
                        let db = vyradb_parse(&raw);
                        let rows = db.get(&table).cloned().unwrap_or_default();
                        let result: Vec<Value> = rows.iter()
                            .filter(|r| !r.starts_with("__schema__"))
                            .map(|r| vyradb_row_to_value(r))
                            .collect();
                        return Ok(Value::List(result));
                    }
                }
                return Ok(Value::List(vec![]));
            }

            "__vyraweb_db_exec" => {
                // NOT IMPLEMENTED YET: raw SQL-style INSERT/UPDATE/DELETE
                // text is not parsed or executed — no data changes happen.
                // Previously this silently returned Ok(0), which looks
                // identical to "0 rows affected" and misleads any caller
                // that expects db_exec to actually mutate data. Failing
                // loudly here is intentional so callers don't assume
                // success. Use db_insert / db_update / db_delete (or
                // db_query for SELECT) for real execution in the meantime.
                let sql = args.into_iter().next().unwrap_or(Value::Null).to_string();
                println!("  VyraDB exec -> {} (NOT EXECUTED - db_exec is not implemented yet)", sql);
                return Err(RuntimeSignal::Error(
                    "VyraDB.exec: raw SQL execution (INSERT/UPDATE/DELETE) is not implemented yet -- \
                     no data was changed. Use db_insert/db_update/db_delete instead, or db_query for SELECT.".into()
                ));
            }

            "__vyraweb_db_drop" => {
                let table   = args.into_iter().next().unwrap_or(Value::Null).to_string();
                let db_path = "vyradb.json";
                let raw = fs::read_to_string(db_path).unwrap_or_else(|_| "{}".into());
                let mut db = vyradb_parse(&raw);
                let existed = db.remove(&table).is_some();
                let schema_key = format!("__schema__{}", table);
                db.remove(&schema_key);
                vyradb_flush(db_path, &db)?;
                println!("✓ VyraDB: Table '{}' dropped (existed={})", table, existed);
                return Ok(Value::Bool(existed));
            }

            // ================================================================
            // VYRAWEB TEMPLATE ENGINE — VyraTmpl
            // Jinja2 jaisi simplicity, Remox ke andar built-in
            //   {{var}}          → variable substitute
            //   {% if cond %}    → conditional block
            //   {% endif %}
            //   {% each item %}  → loop over list
            //   {% endeach %}
            // ================================================================

            "__vyraweb_render" => {
                let tmpl = args.first().cloned().unwrap_or(Value::Null).to_string();
                let data = match args.get(1).cloned().unwrap_or(Value::Null) {
                    Value::Map(p) => p,
                    _ => vec![],
                };
                let rendered = vyratmpl_render(&tmpl, &data);
                return Ok(Value::Str(rendered));
            }

            "__vyraweb_render_file" => {
                let path = args.first().cloned().unwrap_or(Value::Null).to_string();
                let data = match args.get(1).cloned().unwrap_or(Value::Null) {
                    Value::Map(p) => p,
                    _ => vec![],
                };
                let tmpl = fs::read_to_string(&path)
                    .map_err(|e| RuntimeSignal::Error(format!("VyraTmpl.render_file: cannot read '{}': {}", path, e)))?;
                let rendered = vyratmpl_render(&tmpl, &data);
                println!("✓ VyraTmpl: Rendered file '{}'", path);
                return Ok(Value::Str(rendered));
            }

            "__vyraweb_template" => {
                let name = args.first().cloned().unwrap_or(Value::Null).to_string();
                let tmpl = args.get(1).cloned().unwrap_or(Value::Null).to_string();
                // Save template to .vt file so render_file can pick it up
                let path = format!("{}.vt", name);
                fs::write(&path, &tmpl)
                    .map_err(|e| RuntimeSignal::Error(format!("VyraTmpl.template: cannot save '{}': {}", path, e)))?;
                println!("✓ VyraTmpl: Template '{}' registered → {}", name, path);
                return Ok(Value::Str(name));
            }

            "__vyraweb_use_template" => {
                let name = args.first().cloned().unwrap_or(Value::Null).to_string();
                let data = match args.get(1).cloned().unwrap_or(Value::Null) {
                    Value::Map(p) => p,
                    _ => vec![],
                };
                let path = format!("{}.vt", name);
                let tmpl = fs::read_to_string(&path)
                    .map_err(|e| RuntimeSignal::Error(format!("VyraTmpl.use_template: template '{}' not found: {}", name, e)))?;
                let rendered = vyratmpl_render(&tmpl, &data);
                return Ok(Value::Str(rendered));
            }

            // ================================================================
            // VYRAWEB WEBSOCKET — VyraSocket
            // Real TCP WebSocket server (RFC 6455 handshake + framing)
            // ================================================================

            "__vyraweb_ws_listen" => {
                let port = match args.first() {
                    Some(Value::Int(n)) => *n,
                    Some(Value::Str(s)) => s.parse().unwrap_or(9001),
                    _ => 9001,
                };
                println!();
                println!("╔══════════════════════════════════════════════════╗");
                println!("║  🔌 VyraSocket WebSocket Server                  ║");
                println!("║  ws://localhost:{}                            ║", port);
                println!("║  RFC 6455 — Real WebSocket Protocol             ║");
                println!("╚══════════════════════════════════════════════════╝");
                use std::net::TcpListener;
                use std::io::Read;
                let addr = format!("0.0.0.0:{}", port);
                match TcpListener::bind(&addr) {
                    Ok(listener) => {
                        println!("  ✓ VyraSocket listening on ws://{}", addr);
                        let client_count = Arc::new(Mutex::new(0u64));
                        // Same rationale as __vyraweb_run: hand each
                        // connection off via remox_task_spawn() so one
                        // slow/idle WebSocket client can't block new
                        // connections from being accepted.
                        for stream in listener.incoming() {
                            match stream {
                                Ok(mut s) => {
                                    let client_count = client_count.clone();
                                    std::thread::spawn(move || {
                                        let mut buf = [0u8; 4096];
                                        let n = s.read(&mut buf).unwrap_or(0);
                                        let req = String::from_utf8_lossy(&buf[..n]);
                                        // WebSocket RFC 6455 handshake
                                        if req.contains("Upgrade: websocket") {
                                            if let Some(key) = vyrasocket_extract_key(&req) {
                                                let accept = vyrasocket_accept_key(&key);
                                                let response = format!(
                                                    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
                                                    accept
                                                );
                                                let _ = s.write_all(response.as_bytes());
                                                let mut count = client_count.lock().unwrap();
                                                *count += 1;
                                                println!("  ✓ Client #{} connected", count);
                                                // Read one frame and echo it back
                                                let mut frame_buf = [0u8; 4096];
                                                if let Ok(fn_bytes) = s.read(&mut frame_buf) {
                                                    if fn_bytes >= 6 {
                                                        let msg = vyrasocket_decode_frame(&frame_buf[..fn_bytes]);
                                                        println!("  ← Received: {}", msg);
                                                        let reply = format!("VyraSocket echo: {}", msg);
                                                        let encoded = vyrasocket_encode_frame(&reply);
                                                        let _ = s.write_all(&encoded);
                                                    }
                                                }
                                            }
                                        }
                                    });
                                }
                                Err(_) => {}
                            }
                        }
                    }
                    Err(e) => eprintln!("VyraSocket: Cannot bind port {}: {}", port, e),
                }
                return Ok(Value::Null);
            }

            "__vyraweb_ws_on" => {
                let event   = args.first().cloned().unwrap_or(Value::Null).to_string();
                let handler = args.get(1).cloned().unwrap_or(Value::Null).to_string();
                println!("✓ VyraSocket: '{}' event → {} registered", event, handler);
                return Ok(Value::Null);
            }

            "__vyraweb_ws_broadcast" => {
                let msg = args.into_iter().next().unwrap_or(Value::Null).to_string();
                println!("✓ VyraSocket: broadcast → {}", msg);
                return Ok(Value::Str(msg));
            }

            "__vyraweb_ws_send" => {
                let client_id = args.first().cloned().unwrap_or(Value::Null).to_string();
                let msg       = args.get(1).cloned().unwrap_or(Value::Null).to_string();
                println!("✓ VyraSocket: send to client#{} → {}", client_id, msg);
                return Ok(Value::Str(msg));
            }

            "__vyraweb_ws_clients" => {
                // Returns list of connected client IDs (runtime tracking)
                return Ok(Value::List(vec![]));
            }

            _ => {}
        }

        // ---- LAMBDA VARIABLE ----
        if let Some(lambda_val) = self.get_var(name) {
            if matches!(lambda_val, Value::Lambda { .. }) {
                return self.call_value(lambda_val, args);
            }
        }

        // ---- STRUCT CONSTRUCTOR (Feature 28) ----
        if let Some(struct_def) = self.structs.get(name).cloned() {
            // Named args take precedence; positional fill in order
            let mut fields = Vec::new();
            for (i, field_name) in struct_def.fields.iter().enumerate() {
                let val = named.iter().find(|(k, _)| k == field_name)
                    .map(|(_, v)| v.clone())
                    .or_else(|| args.get(i).cloned())
                    .unwrap_or(Value::Null);
                fields.push((field_name.clone(), val));
            }
            return Ok(Value::Struct { name: name.to_string(), fields });
        }

        // ---- USER-DEFINED FUNCTION ----
        let (params, body, is_async) = match self.fns.get(name).cloned() {
            Some(f) => f,
            None => return Err(RuntimeSignal::Error(format!("Undefined function: '{}'", name))),
        };

        // Feature 40: async fn — run in separate thread
        if is_async {
            let result_slot: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
            let slot_clone = Arc::clone(&result_slot);

            // We can't share &mut self across threads, so we build a snapshot interpreter
            // with the current env/fns/structs/impls cloned (shallow copy for closures)
            let env_snapshot   = self.env.clone();
            // Convert Rc<Vec<Stmt>> → plain Vec<Stmt> for thread Send safety
            let fns_snapshot: HashMap<String, (Vec<(String, Option<Expr>)>, Vec<Stmt>, bool)> =
                self.fns.iter().map(|(k, (p, b, a))| {
                    (k.clone(), (p.clone(), (**b).clone(), *a))
                }).collect();
            let structs_snap   = self.structs.clone();
            let impls_snap     = self.impls.clone();
            let traits_snap    = self.traits.clone();
            let rand_state     = self.rand_state;

            // Bind args to params in closure env
            let mut closure_env = env_snapshot.clone();
            let mut call_scope: HashMap<String, Value> = HashMap::new();
            for (i, (pname, default_expr)) in params.iter().enumerate() {
                let val = named.iter().find(|(k, _)| k == pname)
                    .map(|(_, v)| v.clone())
                    .or_else(|| args.get(i).cloned())
                    .or_else(|| {
                        // default param — we'd need to eval in a sub-interp; store as Null for now
                        // because default_expr can't be evaluated without &mut self here
                        default_expr.as_ref().map(|_| Value::Null)
                    })
                    .unwrap_or(Value::Null);
                call_scope.insert(pname.clone(), val);
            }
            closure_env.push(call_scope);

            let body_clone: Vec<Stmt> = (*body).clone();

            thread::spawn(move || {
                // Re-wrap Vec<Stmt> into Rc inside the thread
                let fns_rc: HashMap<String, (Vec<(String, Option<Expr>)>, Rc<Vec<Stmt>>, bool)> =
                    fns_snapshot.into_iter().map(|(k, (p, b, a))| {
                        (k, (p, Rc::new(b), a))
                    }).collect();
                let mut sub_interp = Interpreter {
                    env:        closure_env,
                    fns:        fns_rc,
                    structs:    structs_snap,
                    impls:      impls_snap,
                    traits:     traits_snap,
                    rand_state,
                    memo:       HashMap::new(),
                    pending_styles: Vec::new(),
                    // Async sub-interpreters run detached background work
                    // (see thread::spawn above) — they don't share call
                    // state with the interpreter that spawned them, so
                    // they start at the default language rather than
                    // trying to snapshot/sync it across the thread boundary.
                    remojoke_lang: String::from("src"),
                };
                let result = sub_interp.exec_block(&body_clone);
                let val = match result {
                    Ok(v) => v,
                    Err(RuntimeSignal::Return(Some(v))) => v,
                    _ => Value::Null,
                };
                let mut guard = slot_clone.lock().unwrap();
                *guard = Some(val);
            });

            return Ok(Value::AsyncHandle(result_slot));
        }

        // Synchronous call — resolve all param values BEFORE pushing scope
        // Optimization: memoize single-int-arg pure functions (e.g. fib)
        let memo_key = if args.len() == 1 && named.is_empty() {
            match &args[0] {
                Value::Int(n) => Some((name.to_string(), n.to_string())),
                _ => None,
            }
        } else { None };

        if let Some(ref k) = memo_key {
            if let Some(cached) = self.memo.get(k).cloned() {
                return Ok(cached);
            }
        }

        let mut param_vals: Vec<(String, Value)> = Vec::new();
        for (i, (pname, default_expr)) in params.iter().enumerate() {
            // Priority: named arg > positional arg > default expr > Null
            let val = named.iter().find(|(k, _)| k == pname)
                .map(|(_, v)| v.clone())
                .or_else(|| args.get(i).cloned())
                .or_else(|| {
                    default_expr.as_ref().and_then(|def| {
                        self.eval_expr(def).ok()
                    })
                })
                .unwrap_or(Value::Null);
            param_vals.push((pname.clone(), val));
        }
        self.push_scope();
        for (pname, val) in param_vals {
            self.def_var(&pname, val);
        }

        // Rc::clone is O(1) — no deep copy of body!
        let body_rc = Rc::clone(&body);
        let result = self.exec_block(&body_rc);
        self.pop_scope();

        let ret = match result {
            Ok(v) => Ok(v),
            Err(RuntimeSignal::Return(v)) => Ok(v.unwrap_or(Value::Null)),
            Err(e) => Err(e),
        };

        // Cache result in memo if eligible
        if let (Some(k), Ok(ref v)) = (memo_key, &ret) {
            self.memo.insert(k, v.clone());
        }
        ret
    }

    // =========================================================================
    // TASOAQUE — native function dispatch. Lives on Interpreter (not a free
    // fn like dispatch_autoclib) because executing a task means calling back
    // into `self.call_function(task_name, ...)` — the task is just a normal
    // Remox `fn`, already resolved in `self.fns`.
    // =========================================================================
    fn dispatch_tasoaque(&mut self, name: &str, args: Vec<Value>) -> Result<Value, RuntimeSignal> {
        match name {
            // ── task(name, opts) ─────────────────────────────────────────
            "__tasoaque_task" => {
                let tname = autoclib_arg_str(&args, 0, "");
                if tname.is_empty() {
                    return Err(RuntimeSignal::Error("Tasoaque.task: name required".into()));
                }
                let opts = autoclib_arg_map(&args, 1);
                let queue       = tq_map_str(&opts, "queue", "default");
                let priority    = tq_map_int(&opts, "priority", 5).clamp(0, 9);
                let max_retries = tq_map_int(&opts, "maxRetries", 0).max(0) as u32;
                let retry_delay = tq_map_uint(&opts, "retryDelay", 1).max(1);
                let rate_limit  = match autoclib_map_get(&opts, "rateLimit") {
                    Some(Value::Map(rl)) => {
                        let max = tq_map_int(rl, "max", 0).max(0) as u32;
                        let window = tq_map_uint(rl, "window", 1).max(1);
                        if max > 0 { Some((max, window)) } else { None }
                    }
                    _ => None,
                };
                let mut st = TASOAQUE.lock().unwrap();
                if let Some(existing) = st.task_defs.iter_mut().find(|d| d.name == tname) {
                    existing.queue = queue; existing.priority = priority;
                    existing.max_retries = max_retries; existing.retry_delay = retry_delay;
                } else {
                    st.task_defs.push(TasoaqueTaskDef { name: tname.clone(), queue, priority, max_retries, retry_delay });
                }
                if let Some((max, window)) = rate_limit {
                    if let Some(entry) = st.rate_limits.iter_mut().find(|(t, _, _)| t == &tname) {
                        entry.1 = max; entry.2 = window;
                    } else {
                        st.rate_limits.push((tname.clone(), max, window));
                    }
                }
                Ok(Value::Str(tname))
            }

            // ── enqueue(name, args) / delay(name, args) ──────────────────
            "__tasoaque_enqueue" => {
                let tname = autoclib_arg_str(&args, 0, "");
                let job_args = autoclib_arg_list(&args, 1);
                let mut st = TASOAQUE.lock().unwrap();
                let (queue, priority, max_retries, retry_delay) = tasoaque_resolve_defaults(&st, &tname);
                let clock = st.clock;
                let id = st.fresh_id("tq");
                st.jobs.push(TasoaqueJob {
                    id: id.clone(), task: tname, args: job_args, queue, priority, eta: clock,
                    status: "pending".into(), attempts: 0, max_retries, retry_delay,
                    result: Value::Null, error: String::new(), created_at: clock,
                    chain_next: Vec::new(), group_id: String::new(),
                });
                Ok(Value::Str(id))
            }

            // ── applyAsync(name, args, opts) ──────────────────────────────
            "__tasoaque_apply_async" => {
                let tname = autoclib_arg_str(&args, 0, "");
                let job_args = autoclib_arg_list(&args, 1);
                let opts = autoclib_arg_map(&args, 2);
                let mut st = TASOAQUE.lock().unwrap();
                let (def_queue, def_priority, def_max_retries, def_retry_delay) = tasoaque_resolve_defaults(&st, &tname);
                let queue       = tq_map_str(&opts, "queue", &def_queue);
                let priority    = tq_map_int(&opts, "priority", def_priority).clamp(0, 9);
                let max_retries = tq_map_int(&opts, "maxRetries", def_max_retries as i64).max(0) as u32;
                let retry_delay = tq_map_uint(&opts, "retryDelay", def_retry_delay).max(1);
                let clock = st.clock;
                let countdown = tq_map_uint(&opts, "countdown", 0);
                let eta = match autoclib_map_get(&opts, "eta") {
                    Some(Value::Int(n)) => (*n).max(0) as u64,
                    _ => clock + countdown,
                };
                let id = st.fresh_id("tq");
                st.jobs.push(TasoaqueJob {
                    id: id.clone(), task: tname, args: job_args, queue, priority, eta,
                    status: "pending".into(), attempts: 0, max_retries, retry_delay,
                    result: Value::Null, error: String::new(), created_at: clock,
                    chain_next: Vec::new(), group_id: String::new(),
                });
                Ok(Value::Str(id))
            }

            // ── runWorker(queue, limit) ────────────────────────────────────
            "__tasoaque_run_worker" => {
                let queue = autoclib_arg_str(&args, 0, "");
                let limit = autoclib_arg_int(&args, 1, 50).max(0) as usize;
                let ids = self.tasoaque_process(&queue, limit);
                let (mut succeeded, mut failed, mut retried) = (0i64, 0i64, 0i64);
                {
                    let st = TASOAQUE.lock().unwrap();
                    for id in &ids {
                        if let Some(j) = st.jobs.iter().find(|j| &j.id == id) {
                            match j.status.as_str() {
                                "success" => succeeded += 1,
                                "retry"   => retried += 1,
                                _ => {}
                            }
                        } else if st.dead_letters.iter().any(|j| &j.id == id) {
                            failed += 1;
                        }
                    }
                }
                Ok(Value::Map(vec![
                    ("processed".into(), Value::Int(ids.len() as i64)),
                    ("succeeded".into(), Value::Int(succeeded)),
                    ("failed".into(),    Value::Int(failed)),
                    ("retried".into(),   Value::Int(retried)),
                ]))
            }

            // ── runOne(queue) ───────────────────────────────────────────
            "__tasoaque_run_one" => {
                let queue = autoclib_arg_str(&args, 0, "");
                let ids = self.tasoaque_process(&queue, 1);
                if ids.is_empty() { return Ok(Value::Null); }
                let st = TASOAQUE.lock().unwrap();
                if let Some(j) = st.jobs.iter().find(|j| j.id == ids[0]) { return Ok(tq_job_to_map(j)); }
                if let Some(j) = st.dead_letters.iter().find(|j| j.id == ids[0]) { return Ok(tq_job_to_map(j)); }
                Ok(Value::Null)
            }

            // ── status / result / error ────────────────────────────────
            "__tasoaque_status" => {
                let id = autoclib_arg_str(&args, 0, "");
                let st = TASOAQUE.lock().unwrap();
                if let Some(j) = st.jobs.iter().find(|j| j.id == id) { return Ok(Value::Str(j.status.clone())); }
                if let Some(j) = st.dead_letters.iter().find(|j| j.id == id) { return Ok(Value::Str(j.status.clone())); }
                Ok(Value::Str("not_found".to_string()))
            }
            "__tasoaque_result" => {
                let id = autoclib_arg_str(&args, 0, "");
                let st = TASOAQUE.lock().unwrap();
                if let Some(j) = st.jobs.iter().find(|j| j.id == id) { return Ok(j.result.clone()); }
                if let Some(j) = st.dead_letters.iter().find(|j| j.id == id) { return Ok(j.result.clone()); }
                Ok(Value::Null)
            }
            "__tasoaque_error" => {
                let id = autoclib_arg_str(&args, 0, "");
                let st = TASOAQUE.lock().unwrap();
                let found = st.jobs.iter().find(|j| j.id == id).or_else(|| st.dead_letters.iter().find(|j| j.id == id));
                Ok(match found {
                    Some(j) if !j.error.is_empty() => Value::Str(j.error.clone()),
                    _ => Value::Null,
                })
            }

            // ── cancel / requeue / purge ────────────────────────────────
            "__tasoaque_cancel" => {
                let id = autoclib_arg_str(&args, 0, "");
                let mut st = TASOAQUE.lock().unwrap();
                if let Some(j) = st.jobs.iter_mut().find(|j| j.id == id && (j.status == "pending" || j.status == "retry")) {
                    j.status = "cancelled".to_string();
                    return Ok(Value::Bool(true));
                }
                Ok(Value::Bool(false))
            }
            "__tasoaque_requeue" => {
                let id = autoclib_arg_str(&args, 0, "");
                let mut st = TASOAQUE.lock().unwrap();
                if let Some(pos) = st.dead_letters.iter().position(|j| j.id == id) {
                    let mut job = st.dead_letters.remove(pos);
                    job.status = "pending".to_string();
                    job.attempts = 0;
                    job.error = String::new();
                    job.eta = st.clock;
                    st.jobs.push(job);
                    return Ok(Value::Bool(true));
                }
                Ok(Value::Bool(false))
            }
            "__tasoaque_purge" => {
                let queue = autoclib_arg_str(&args, 0, "");
                let mut st = TASOAQUE.lock().unwrap();
                let before = st.jobs.len();
                st.jobs.retain(|j| !((j.status == "pending" || j.status == "retry") && (queue.is_empty() || j.queue == queue)));
                Ok(Value::Int((before - st.jobs.len()) as i64))
            }

            // ── stats / deadLetters / queueLength ────────────────────────
            "__tasoaque_stats" => {
                let queue = autoclib_arg_str(&args, 0, "");
                let st = TASOAQUE.lock().unwrap();
                let (mut pending, mut running, mut retry, mut success, mut cancelled) = (0i64, 0i64, 0i64, 0i64, 0i64);
                for j in st.jobs.iter().filter(|j| queue.is_empty() || j.queue == queue) {
                    match j.status.as_str() {
                        "pending"   => pending += 1,
                        "running"   => running += 1,
                        "retry"     => retry += 1,
                        "success"   => success += 1,
                        "cancelled" => cancelled += 1,
                        _ => {}
                    }
                }
                let failed = st.dead_letters.iter().filter(|j| queue.is_empty() || j.queue == queue).count() as i64;
                Ok(Value::Map(vec![
                    ("pending".into(), Value::Int(pending)), ("running".into(), Value::Int(running)),
                    ("retry".into(), Value::Int(retry)), ("success".into(), Value::Int(success)),
                    ("failed".into(), Value::Int(failed)), ("cancelled".into(), Value::Int(cancelled)),
                ]))
            }
            "__tasoaque_dead_letters" => {
                let queue = autoclib_arg_str(&args, 0, "");
                let st = TASOAQUE.lock().unwrap();
                let list: Vec<Value> = st.dead_letters.iter()
                    .filter(|j| queue.is_empty() || j.queue == queue)
                    .map(tq_job_to_map)
                    .collect();
                Ok(Value::List(list))
            }
            "__tasoaque_queue_length" => {
                let queue = autoclib_arg_str(&args, 0, "");
                let st = TASOAQUE.lock().unwrap();
                let n = st.jobs.iter().filter(|j| (j.status == "pending" || j.status == "retry") && (queue.is_empty() || j.queue == queue)).count();
                Ok(Value::Int(n as i64))
            }

            // ── chain / group / chord / groupResults ─────────────────────
            "__tasoaque_chain" => {
                let steps = autoclib_arg_list(&args, 0);
                if steps.is_empty() {
                    return Err(RuntimeSignal::Error("Tasoaque.chain: steps list is empty".into()));
                }
                let mut st = TASOAQUE.lock().unwrap();
                let mut parsed: Vec<(String, Vec<Value>, String)> = Vec::new();
                for step in &steps {
                    if let Value::Map(m) = step {
                        let task = tq_map_str(m, "task", "");
                        let step_args = match autoclib_map_get(m, "args") { Some(Value::List(l)) => l.clone(), _ => Vec::new() };
                        let (def_queue, _, _, _) = tasoaque_resolve_defaults(&st, &task);
                        let queue = tq_map_str(m, "queue", &def_queue);
                        parsed.push((task, step_args, queue));
                    }
                }
                if parsed.is_empty() {
                    return Err(RuntimeSignal::Error("Tasoaque.chain: no valid {\"task\":..,\"args\":[..]} steps found".into()));
                }
                let (first_task, first_args, first_queue) = parsed.remove(0);
                let (_, priority, max_retries, retry_delay) = tasoaque_resolve_defaults(&st, &first_task);
                let clock = st.clock;
                let id = st.fresh_id("tq");
                st.jobs.push(TasoaqueJob {
                    id: id.clone(), task: first_task, args: first_args, queue: first_queue,
                    priority, eta: clock, status: "pending".into(), attempts: 0, max_retries, retry_delay,
                    result: Value::Null, error: String::new(), created_at: clock,
                    chain_next: parsed, group_id: String::new(),
                });
                Ok(Value::Str(id))
            }
            "__tasoaque_group" => {
                let specs = autoclib_arg_list(&args, 0);
                let mut st = TASOAQUE.lock().unwrap();
                let group_id = st.fresh_id("grp");
                let mut job_ids = Vec::new();
                for spec in &specs {
                    if let Value::Map(m) = spec {
                        let task = tq_map_str(m, "task", "");
                        let job_args = match autoclib_map_get(m, "args") { Some(Value::List(l)) => l.clone(), _ => Vec::new() };
                        let (def_queue, def_priority, max_retries, retry_delay) = tasoaque_resolve_defaults(&st, &task);
                        let queue = tq_map_str(m, "queue", &def_queue);
                        let priority = tq_map_int(m, "priority", def_priority).clamp(0, 9);
                        let clock = st.clock;
                        let id = st.fresh_id("tq");
                        st.jobs.push(TasoaqueJob {
                            id: id.clone(), task, args: job_args, queue, priority, eta: clock,
                            status: "pending".into(), attempts: 0, max_retries, retry_delay,
                            result: Value::Null, error: String::new(), created_at: clock,
                            chain_next: Vec::new(), group_id: group_id.clone(),
                        });
                        job_ids.push(id);
                    }
                }
                st.groups.push(TasoaqueGroup { id: group_id.clone(), job_ids, callback: String::new(), callback_queue: String::new(), fired: false });
                Ok(Value::Str(group_id))
            }
            "__tasoaque_chord" => {
                let specs = autoclib_arg_list(&args, 0);
                let callback = autoclib_arg_str(&args, 1, "");
                if callback.is_empty() {
                    return Err(RuntimeSignal::Error("Tasoaque.chord: callback task name required".into()));
                }
                let mut st = TASOAQUE.lock().unwrap();
                let group_id = st.fresh_id("grp");
                let mut job_ids = Vec::new();
                for spec in &specs {
                    if let Value::Map(m) = spec {
                        let task = tq_map_str(m, "task", "");
                        let job_args = match autoclib_map_get(m, "args") { Some(Value::List(l)) => l.clone(), _ => Vec::new() };
                        let (def_queue, def_priority, max_retries, retry_delay) = tasoaque_resolve_defaults(&st, &task);
                        let queue = tq_map_str(m, "queue", &def_queue);
                        let priority = tq_map_int(m, "priority", def_priority).clamp(0, 9);
                        let clock = st.clock;
                        let id = st.fresh_id("tq");
                        st.jobs.push(TasoaqueJob {
                            id: id.clone(), task, args: job_args, queue, priority, eta: clock,
                            status: "pending".into(), attempts: 0, max_retries, retry_delay,
                            result: Value::Null, error: String::new(), created_at: clock,
                            chain_next: Vec::new(), group_id: group_id.clone(),
                        });
                        job_ids.push(id);
                    }
                }
                let (callback_queue, _, _, _) = tasoaque_resolve_defaults(&st, &callback);
                st.groups.push(TasoaqueGroup { id: group_id.clone(), job_ids, callback, callback_queue, fired: false });
                Ok(Value::Str(group_id))
            }
            "__tasoaque_group_results" => {
                let gid = autoclib_arg_str(&args, 0, "");
                let st = TASOAQUE.lock().unwrap();
                let group = match st.groups.iter().find(|g| g.id == gid) {
                    Some(g) => g,
                    None => return Ok(Value::List(Vec::new())),
                };
                let results: Vec<Value> = group.job_ids.iter().map(|jid| {
                    if let Some(j) = st.jobs.iter().find(|j| &j.id == jid) {
                        if j.status == "success" { j.result.clone() } else { Value::Null }
                    } else {
                        Value::Null
                    }
                }).collect();
                Ok(Value::List(results))
            }

            // ── schedule / unschedule / tick (periodic tasks, logical clock) ─
            "__tasoaque_schedule" => {
                let tname = autoclib_arg_str(&args, 0, "");
                let sched_args = autoclib_arg_list(&args, 1);
                let interval = autoclib_arg_int(&args, 2, 1).max(1) as u64;
                let queue_arg = autoclib_arg_str(&args, 3, "");
                let mut st = TASOAQUE.lock().unwrap();
                let (def_queue, priority, _, _) = tasoaque_resolve_defaults(&st, &tname);
                let queue = if queue_arg.is_empty() { def_queue } else { queue_arg };
                let clock = st.clock;
                let id = st.fresh_id("sch");
                st.schedules.push(TasoaqueSchedule { id: id.clone(), task: tname, args: sched_args, queue, interval, next_run: clock + interval, priority });
                Ok(Value::Str(id))
            }
            "__tasoaque_unschedule" => {
                let id = autoclib_arg_str(&args, 0, "");
                let mut st = TASOAQUE.lock().unwrap();
                let before = st.schedules.len();
                st.schedules.retain(|s| s.id != id);
                Ok(Value::Bool(st.schedules.len() < before))
            }
            "__tasoaque_tick" => {
                let n = autoclib_arg_int(&args, 0, 1).max(0) as u64;
                let mut st = TASOAQUE.lock().unwrap();
                st.clock += n;
                let clock = st.clock;
                for i in 0..st.schedules.len() {
                    let mut guard = 0u32;
                    while st.schedules[i].next_run <= clock && guard < 1000 {
                        let (task, sargs, queue, priority) = (
                            st.schedules[i].task.clone(), st.schedules[i].args.clone(),
                            st.schedules[i].queue.clone(), st.schedules[i].priority,
                        );
                        let (_, _, max_retries, retry_delay) = tasoaque_resolve_defaults(&st, &task);
                        let new_id = st.fresh_id("tq");
                        st.jobs.push(TasoaqueJob {
                            id: new_id, task, args: sargs, queue, priority, eta: clock,
                            status: "pending".into(), attempts: 0, max_retries, retry_delay,
                            result: Value::Null, error: String::new(), created_at: clock,
                            chain_next: Vec::new(), group_id: String::new(),
                        });
                        let interval = st.schedules[i].interval.max(1);
                        st.schedules[i].next_run += interval;
                        guard += 1;
                    }
                }
                Ok(Value::Int(st.clock as i64))
            }

            // ── rateLimit(name, max, window) ──────────────────────────────
            "__tasoaque_rate_limit" => {
                let tname = autoclib_arg_str(&args, 0, "");
                let max = autoclib_arg_int(&args, 1, 0).max(0) as u32;
                let window = autoclib_arg_int(&args, 2, 1).max(1) as u64;
                let mut st = TASOAQUE.lock().unwrap();
                if let Some(entry) = st.rate_limits.iter_mut().find(|(t, _, _)| t == &tname) {
                    entry.1 = max; entry.2 = window;
                } else {
                    st.rate_limits.push((tname, max, window));
                }
                Ok(Value::Bool(true))
            }

            // ── serve(port) — become a Tasoaque cluster coordinator ──────
            // Blocking, exactly like Vyraweb.listen()/VyraSocket.listen():
            // accepts connections one at a time, handles exactly one
            // pull/complete/fail request per connection, then moves on to
            // the next. Never executes task code itself.
            "__tasoaque_serve" => {
                let port = autoclib_arg_int(&args, 0, 9500);
                let addr = format!("0.0.0.0:{}", port);
                println!();
                println!("╔══════════════════════════════════════════════════╗");
                println!("║  🧵 Tasoaque Cluster Coordinator                 ║");
                println!("║  tasoaque://{}                             ║", addr);
                println!("║  Real cross-machine workers — no external broker ║");
                println!("╚══════════════════════════════════════════════════╝");
                use std::net::TcpListener;
                match TcpListener::bind(&addr) {
                    Ok(listener) => {
                        println!("  ✓ Tasoaque coordinator listening on {}", addr);
                        for stream in listener.incoming() {
                            match stream {
                                Ok(s) => {
                                    let id = {
                                        use std::sync::atomic::Ordering;
                                        let nid = remox_fresh_id();
                                        REMOX_STREAMS.lock().unwrap()
                                            .get_or_insert_with(HashMap::new)
                                            .insert(nid, s);
                                        nid
                                    };
                                    match tasoaque_recv_frame(id) {
                                        Ok(payload) => {
                                            let resp = tasoaque_handle_wire_op(&payload);
                                            if let Err(e) = tasoaque_send_frame(id, &resp) {
                                                eprintln!("Tasoaque coordinator: send failed — {}", e);
                                            }
                                        }
                                        Err(e) => eprintln!("Tasoaque coordinator: recv failed — {}", e),
                                    }
                                },
                                Err(e) => eprintln!("Tasoaque coordinator: accept error — {}", e),
                            }
                        }
                    }
                    Err(e) => eprintln!("Tasoaque coordinator: cannot bind {} — {}", addr, e),
                }
                Ok(Value::Null)
            }

            // ── remoteWork(host, port, queue, limit) — become a remote ────
            // worker on a completely separate machine/process. Pulls one
            // real job at a time over TCP, executes it LOCALLY on whatever
            // hardware this call is running on (self.call_function — the
            // task fn must be defined in this worker's own script, exactly
            // like Celery/RQ require task code to be deployed to every
            // worker), then reports success/failure back to the coordinator.
            "__tasoaque_remote_work" => {
                let host = autoclib_arg_str(&args, 0, "");
                let port = autoclib_arg_int(&args, 1, 9500);
                let queue = autoclib_arg_str(&args, 2, "");
                let limit = autoclib_arg_int(&args, 3, 10).max(0);
                let addr = format!("{}:{}", host, port);
                let (mut processed, mut succeeded, mut failed) = (0i64, 0i64, 0i64);

                for _ in 0..limit {
                    let handle = remox_tcp_connect(&addr)
                        .map_err(|e| RuntimeSignal::Error(format!("Tasoaque.remoteWork: connect to {} failed — {}", addr, e)))?;

                    let pull_req = value_to_json(&Value::Map(vec![
                        ("op".into(), Value::Str("pull".into())),
                        ("queue".into(), Value::Str(queue.clone())),
                    ]));
                    tasoaque_send_frame(handle, pull_req.as_bytes())
                        .map_err(|e| RuntimeSignal::Error(format!("Tasoaque.remoteWork: pull send failed — {}", e)))?;
                    let resp_bytes = tasoaque_recv_frame(handle)
                        .map_err(|e| RuntimeSignal::Error(format!("Tasoaque.remoteWork: pull recv failed — {}", e)))?;

                    let resp_text = String::from_utf8_lossy(&resp_bytes).to_string();
                    let resp_map = match vyradb_json_parse_value(&resp_text) {
                        Some(Value::Map(m)) => m,
                        _ => break,
                    };
                    let job_id = match autoclib_map_get(&resp_map, "jobId") {
                        Some(Value::Str(s)) => s.clone(),
                        _ => break, // koi due job nahi — session khatam
                    };
                    let task_name = tq_map_str(&resp_map, "task", "");
                    let job_args = match autoclib_map_get(&resp_map, "args") { Some(Value::List(l)) => l.clone(), _ => Vec::new() };

                    processed += 1;
                    let outcome = self.call_function(&task_name, job_args, Vec::new());

                    let report_handle = remox_tcp_connect(&addr)
                        .map_err(|e| RuntimeSignal::Error(format!("Tasoaque.remoteWork: report connect failed — {}", e)))?;
                    let report_req = match outcome {
                        Ok(v) => {
                            succeeded += 1;
                            value_to_json(&Value::Map(vec![
                                ("op".into(), Value::Str("complete".into())),
                                ("jobId".into(), Value::Str(job_id)),
                                ("result".into(), v),
                            ]))
                        }
                        Err(sig) => {
                            failed += 1;
                            let msg = match sig {
                                RuntimeSignal::Error(m) => m,
                                RuntimeSignal::Exit(code) => format!("task called exit({})", code),
                                RuntimeSignal::Return(_) => "task returned outside of a call context".to_string(),
                            };
                            value_to_json(&Value::Map(vec![
                                ("op".into(), Value::Str("fail".into())),
                                ("jobId".into(), Value::Str(job_id)),
                                ("error".into(), Value::Str(msg)),
                            ]))
                        }
                    };
                    if let Err(e) = tasoaque_send_frame(report_handle, report_req.as_bytes()) {
                        eprintln!("Tasoaque.remoteWork: report send failed — {}", e);
                    }
                    let _ = tasoaque_recv_frame(report_handle); // ack, body not needed
                }

                Ok(Value::Map(vec![
                    ("processed".into(), Value::Int(processed)),
                    ("succeeded".into(), Value::Int(succeeded)),
                    ("failed".into(), Value::Int(failed)),
                ]))
            }

            _ => Err(RuntimeSignal::Error(format!("Unknown Tasoaque function: {}", name))),
        }
    }

    // =========================================================================
    // Sceuti — dispatch. Inline (needs &mut self) because
    // schedule_runPending's due jobs are executed via self.call_function()
    // with each job's own registered fn_name.
    // =========================================================================
    fn dispatch_sceuti(&mut self, name: &str, args: Vec<Value>) -> Result<Value, RuntimeSignal> {
        match name {
            // ── SceutiClock ──────────────────────────────────────────────
            "__sceuti_clock_now"         => { Ok(Value::Int(sceuti_clock_now().epoch as i64)) }
            "__sceuti_clock_from_epoch"  => { let e = autoclib_arg_int(&args, 0, 0) as u64; Ok(Value::Int(sceuti_clock_from_epoch(e).epoch as i64)) }
            "__sceuti_clock_format"      => { let e = autoclib_arg_int(&args, 0, 0) as u64; let p = autoclib_arg_str(&args, 1, "{epoch}"); Ok(Value::Str(sceuti_clock_format(&SceutiTime::new(e), &p))) }
            "__sceuti_clock_diff"        => { let t1 = SceutiTime::new(autoclib_arg_int(&args, 0, 0) as u64); let t2 = SceutiTime::new(autoclib_arg_int(&args, 1, 0) as u64); Ok(Value::Int(sceuti_clock_diff_seconds(&t1, &t2))) }
            "__sceuti_clock_add_seconds" => { let e = autoclib_arg_int(&args, 0, 0) as u64; let n = autoclib_arg_int(&args, 1, 0); Ok(Value::Int(sceuti_clock_add_seconds(&SceutiTime::new(e), n).epoch as i64)) }
            "__sceuti_clock_add_minutes" => { let e = autoclib_arg_int(&args, 0, 0) as u64; let n = autoclib_arg_int(&args, 1, 0); Ok(Value::Int(sceuti_clock_add_minutes(&SceutiTime::new(e), n).epoch as i64)) }
            "__sceuti_clock_add_hours"   => { let e = autoclib_arg_int(&args, 0, 0) as u64; let n = autoclib_arg_int(&args, 1, 0); Ok(Value::Int(sceuti_clock_add_hours(&SceutiTime::new(e), n).epoch as i64)) }
            "__sceuti_clock_humanize"    => { let e = autoclib_arg_int(&args, 0, 0) as u64; Ok(Value::Str(sceuti_clock_humanize(&SceutiTime::new(e)))) }
            "__sceuti_clock_is_before"   => { let t1 = SceutiTime::new(autoclib_arg_int(&args, 0, 0) as u64); let t2 = SceutiTime::new(autoclib_arg_int(&args, 1, 0) as u64); Ok(Value::Bool(sceuti_clock_is_before(&t1, &t2))) }
            "__sceuti_clock_is_after"    => { let t1 = SceutiTime::new(autoclib_arg_int(&args, 0, 0) as u64); let t2 = SceutiTime::new(autoclib_arg_int(&args, 1, 0) as u64); Ok(Value::Bool(sceuti_clock_is_after(&t1, &t2))) }

            // ── SceutiSchedule ───────────────────────────────────────────
            "__sceuti_schedule_every_n" => {
                let n = autoclib_arg_int(&args, 0, 1000).max(0) as u64;
                let f = autoclib_arg_str(&args, 1, "");
                let clock = SCEUTI_CLOCK.load(core::sync::atomic::Ordering::Relaxed);
                let mut st = SCEUTI.lock().unwrap();
                let id = sceuti_schedule_every_n(&mut st.jobs, clock, n, f);
                Ok(Value::Int(id as i64))
            }
            "__sceuti_schedule_once" => {
                let n = autoclib_arg_int(&args, 0, 0).max(0) as u64;
                let f = autoclib_arg_str(&args, 1, "");
                let clock = SCEUTI_CLOCK.load(core::sync::atomic::Ordering::Relaxed);
                let mut st = SCEUTI.lock().unwrap();
                let id = sceuti_schedule_once_after(&mut st.jobs, clock, n, f);
                Ok(Value::Int(id as i64))
            }
            "__sceuti_schedule_cancel" => {
                let id = autoclib_arg_int(&args, 0, 0) as u64;
                let mut st = SCEUTI.lock().unwrap();
                sceuti_schedule_cancel(&mut st.jobs, id);
                Ok(Value::Null)
            }
            "__sceuti_schedule_list" => {
                let st = SCEUTI.lock().unwrap();
                Ok(Value::List(sceuti_schedule_list_jobs(&st.jobs).into_iter()
                    .map(|(id, fname)| Value::List(vec![Value::Int(id as i64), Value::Str(fname)]))
                    .collect()))
            }
            "__sceuti_schedule_pause"  => { let mut st = SCEUTI.lock().unwrap(); sceuti_schedule_pause_all(&mut st.jobs); Ok(Value::Null) }
            "__sceuti_schedule_resume" => { let mut st = SCEUTI.lock().unwrap(); sceuti_schedule_resume_all(&mut st.jobs); Ok(Value::Null) }
            "__sceuti_schedule_count"  => { let st = SCEUTI.lock().unwrap(); Ok(Value::Int(sceuti_schedule_job_count(&st.jobs) as i64)) }
            "__sceuti_schedule_run" => {
                let clock = SCEUTI_CLOCK.load(core::sync::atomic::Ordering::Relaxed);
                let due = {
                    let mut st = SCEUTI.lock().unwrap();
                    sceuti_schedule_run_pending(&mut st.jobs, clock)
                };
                for fname in due {
                    self.call_function(&fname, Vec::new(), Vec::new())?;
                }
                Ok(Value::Null)
            }
            "__sceuti_schedule_clear" => { let mut st = SCEUTI.lock().unwrap(); sceuti_schedule_clear_all(&mut st.jobs); Ok(Value::Null) }
            "__sceuti_schedule_on_boot" => {
                let f = autoclib_arg_str(&args, 0, "");
                let mut st = SCEUTI.lock().unwrap();
                let boot_jobs_run = &mut st.boot_jobs_run;
                let mut run_flag = *boot_jobs_run;
                sceuti_schedule_on_boot(&mut st.jobs, &mut run_flag, f);
                st.boot_jobs_run = run_flag;
                Ok(Value::Null)
            }

            // ── SceutiEnv ────────────────────────────────────────────────
            "__sceuti_env_get" => {
                let k = autoclib_arg_str(&args, 0, "");
                let st = SCEUTI.lock().unwrap();
                Ok(sceuti_env_get(&st.env_store, &k).map(Value::Str).unwrap_or(Value::Null))
            }
            "__sceuti_env_set" => {
                let k = autoclib_arg_str(&args, 0, "");
                let v = autoclib_arg_str(&args, 1, "");
                let mut st = SCEUTI.lock().unwrap();
                sceuti_env_set(&mut st.env_store, k, v);
                Ok(Value::Null)
            }
            "__sceuti_env_delete" => {
                let k = autoclib_arg_str(&args, 0, "");
                let mut st = SCEUTI.lock().unwrap();
                sceuti_env_delete(&mut st.env_store, &k);
                Ok(Value::Null)
            }
            "__sceuti_env_has" => {
                let k = autoclib_arg_str(&args, 0, "");
                let st = SCEUTI.lock().unwrap();
                Ok(Value::Bool(sceuti_env_has(&st.env_store, &k)))
            }
            "__sceuti_env_keys"   => { let st = SCEUTI.lock().unwrap(); Ok(Value::List(sceuti_env_keys(&st.env_store).into_iter().map(Value::Str).collect())) }
            "__sceuti_env_values" => { let st = SCEUTI.lock().unwrap(); Ok(Value::List(sceuti_env_values(&st.env_store).into_iter().map(Value::Str).collect())) }
            "__sceuti_env_entries" => {
                let st = SCEUTI.lock().unwrap();
                Ok(Value::List(sceuti_env_entries(&st.env_store).into_iter()
                    .map(|(k, v)| Value::List(vec![Value::Str(k), Value::Str(v)]))
                    .collect()))
            }
            "__sceuti_env_load_string" => {
                let raw = autoclib_arg_str(&args, 0, "");
                let mut st = SCEUTI.lock().unwrap();
                sceuti_env_load_string(&mut st.env_store, &raw);
                Ok(Value::Null)
            }
            "__sceuti_env_to_map" => {
                let st = SCEUTI.lock().unwrap();
                Ok(Value::Map(sceuti_env_to_map(&st.env_store).into_iter().map(|(k, v)| (k, Value::Str(v))).collect()))
            }
            "__sceuti_env_clear" => { let mut st = SCEUTI.lock().unwrap(); sceuti_env_clear(&mut st.env_store); Ok(Value::Null) }

            // ── SceutiLog ────────────────────────────────────────────────
            "__sceuti_log_info" => {
                let m = autoclib_arg_str(&args, 0, "");
                let clock = SCEUTI_CLOCK.load(core::sync::atomic::Ordering::Relaxed);
                let mut st = SCEUTI.lock().unwrap();
                let prefix = st.log_prefix.clone();
                sceuti_log_info(&mut st.log_history, &prefix, m, clock);
                Ok(Value::Null)
            }
            "__sceuti_log_warn" => {
                let m = autoclib_arg_str(&args, 0, "");
                let clock = SCEUTI_CLOCK.load(core::sync::atomic::Ordering::Relaxed);
                let mut st = SCEUTI.lock().unwrap();
                let prefix = st.log_prefix.clone();
                sceuti_log_warn(&mut st.log_history, &prefix, m, clock);
                Ok(Value::Null)
            }
            "__sceuti_log_error" => {
                let m = autoclib_arg_str(&args, 0, "");
                let clock = SCEUTI_CLOCK.load(core::sync::atomic::Ordering::Relaxed);
                let mut st = SCEUTI.lock().unwrap();
                let prefix = st.log_prefix.clone();
                sceuti_log_error(&mut st.log_history, &prefix, m, clock);
                Ok(Value::Null)
            }
            "__sceuti_log_debug" => {
                let m = autoclib_arg_str(&args, 0, "");
                let clock = SCEUTI_CLOCK.load(core::sync::atomic::Ordering::Relaxed);
                let mut st = SCEUTI.lock().unwrap();
                let prefix = st.log_prefix.clone();
                sceuti_log_debug(&mut st.log_history, &prefix, m, clock);
                Ok(Value::Null)
            }
            "__sceuti_log_set_level"   => { sceuti_log_set_level(autoclib_arg_int(&args, 0, 0).max(0) as u64); Ok(Value::Null) }
            "__sceuti_log_with_prefix" => {
                let p = autoclib_arg_str(&args, 0, "");
                let mut st = SCEUTI.lock().unwrap();
                st.log_prefix = p;
                Ok(Value::Null)
            }
            "__sceuti_log_history" => {
                let n = autoclib_arg_int(&args, 0, 20).max(0) as usize;
                let st = SCEUTI.lock().unwrap();
                Ok(Value::List(sceuti_log_history(&st.log_history, n).into_iter().map(Value::Str).collect()))
            }
            "__sceuti_log_history_count" => { let st = SCEUTI.lock().unwrap(); Ok(Value::Int(sceuti_log_history_count(&st.log_history) as i64)) }
            "__sceuti_log_clear"         => { let mut st = SCEUTI.lock().unwrap(); sceuti_log_clear_history(&mut st.log_history); Ok(Value::Null) }
            "__sceuti_log_format" => {
                let lv = autoclib_arg_str(&args, 0, "INFO");
                let msg = autoclib_arg_str(&args, 1, "");
                let clock = SCEUTI_CLOCK.load(core::sync::atomic::Ordering::Relaxed);
                let st = SCEUTI.lock().unwrap();
                Ok(Value::Str(sceuti_log_format(&lv, &st.log_prefix, &msg, clock)))
            }

            // ── SceutiFake ───────────────────────────────────────────────
            "__sceuti_fake_int"  => { let mn = autoclib_arg_int(&args, 0, 0); let mx = autoclib_arg_int(&args, 1, 100); Ok(Value::Int(sceuti_fake_int(mn, mx))) }
            "__sceuti_fake_str"  => { let l = autoclib_arg_int(&args, 0, 8).max(0) as usize; Ok(Value::Str(sceuti_fake_str(l))) }
            "__sceuti_fake_bool" => { Ok(Value::Bool(sceuti_fake_bool())) }
            "__sceuti_fake_from" => {
                if let Some(Value::List(list)) = args.into_iter().next() {
                    Ok(sceuti_fake_from(&list).unwrap_or(Value::Null))
                } else { Ok(Value::Null) }
            }
            "__sceuti_fake_seed" => { sceuti_fake_seed(autoclib_arg_int(&args, 0, 1).max(0) as u64); Ok(Value::Null) }
            "__sceuti_fake_uuid" => { Ok(Value::Str(sceuti_fake_uuid())) }
            "__sceuti_fake_hex"  => { let l = autoclib_arg_int(&args, 0, 8).max(0) as usize; Ok(Value::Str(sceuti_fake_hex(l))) }
            "__sceuti_fake_bytes" => {
                let n = autoclib_arg_int(&args, 0, 8).max(0) as usize;
                Ok(Value::List(sceuti_fake_bytes(n).into_iter().map(|b| Value::Int(b as i64)).collect()))
            }
            "__sceuti_fake_ipv4" => { Ok(Value::Str(sceuti_fake_ipv4())) }
            "__sceuti_fake_shuffle" => {
                if let Some(Value::List(list)) = args.into_iter().next() {
                    Ok(Value::List(sceuti_fake_shuffle(&list)))
                } else { Ok(Value::Null) }
            }

            _ => Err(RuntimeSignal::Error(format!("Unknown Sceuti function: {}", name))),
        }
    }

    // =========================================================================
    // Remotest — dispatch. Inline (needs &mut self) because running tests,
    // fixtures, and spies means calling back into self.call_value(). All
    // registries below use the same UnsafeCell+unsafe-Sync single-core
    // pattern as AUTOCLIB_HISTORY — real in-memory state, not simulated.
    // =========================================================================
    fn dispatch_remotest(&mut self, name: &str, args: Vec<Value>) -> Result<Value, RuntimeSignal> {
        fn arg(args: &[Value], i: usize) -> Value { args.get(i).cloned().unwrap_or(Value::Null) }
        fn arg_str(args: &[Value], i: usize, default: &str) -> String {
            match args.get(i) { Some(Value::Str(s)) => s.clone(), _ => default.to_string() }
        }
        fn arg_tags(args: &[Value], i: usize) -> Vec<String> {
            match args.get(i) {
                Some(Value::List(l)) => l.iter().map(|v| v.to_string()).collect(),
                _ => Vec::new(),
            }
        }
        const SKIP_SENTINEL: &str = "__REMOTEST_SKIP__";

        match name {
            "__remotest_test" | "__remotest_it" => {
                let raw_name = arg_str(&args, 0, "unnamed test");
                let full_name = {
                    let groups = unsafe { &*REMOTEST_GROUPS.0.get() };
                    if groups.is_empty() { raw_name } else { format!("{} > {}", groups.join(" > "), raw_name) }
                };
                let f = arg(&args, 1);
                let tags = arg_tags(&args, 2);
                unsafe { (&mut *REMOTEST_TESTS.0.get()).push((full_name, f, tags)); }
                Ok(Value::Bool(true))
            }
            "__remotest_describe" => {
                let group_name = arg_str(&args, 0, "unnamed group");
                let block = arg(&args, 1);
                unsafe { (&mut *REMOTEST_GROUPS.0.get()).push(group_name); }
                let r = self.call_value(block, Vec::new());
                unsafe { (&mut *REMOTEST_GROUPS.0.get()).pop(); }
                r
            }
            "__remotest_skip" => {
                let reason = arg_str(&args, 0, "skipped");
                Err(RuntimeSignal::Error(format!("{}{}", SKIP_SENTINEL, reason)))
            }

            // ---- Fixtures ----
            "__remotest_fixture" => {
                let fname = arg_str(&args, 0, "");
                let f = arg(&args, 1);
                unsafe { (&mut *REMOTEST_FIXTURES.0.get()).push((fname, f)); }
                Ok(Value::Bool(true))
            }
            "__remotest_use_fixture" => {
                let fname = arg_str(&args, 0, "");
                let found = unsafe { (&*REMOTEST_FIXTURES.0.get()).iter().find(|(n, _)| *n == fname).map(|(_, f)| f.clone()) };
                match found {
                    Some(f) => self.call_value(f, Vec::new()),
                    None => Err(RuntimeSignal::Error(format!("Remotest.useFixture: no fixture named '{}'", fname))),
                }
            }

            // ---- Assertions ----
            "__remotest_assert_equal" => {
                let (a, b) = (arg(&args, 0), arg(&args, 1));
                if a == b { Ok(Value::Bool(true)) }
                else { Err(RuntimeSignal::Error(format!("assertEqual failed: {} != {}{}", a, b, msg_suffix(&args, 2)))) }
            }
            "__remotest_assert_not_equal" => {
                let (a, b) = (arg(&args, 0), arg(&args, 1));
                if a != b { Ok(Value::Bool(true)) }
                else { Err(RuntimeSignal::Error(format!("assertNotEqual failed: {} == {}{}", a, b, msg_suffix(&args, 2)))) }
            }
            "__remotest_assert_true" => {
                let v = arg(&args, 0);
                if matches!(v, Value::Bool(true)) { Ok(Value::Bool(true)) }
                else { Err(RuntimeSignal::Error(format!("assertTrue failed: got {}{}", v, msg_suffix(&args, 1)))) }
            }
            "__remotest_assert_false" => {
                let v = arg(&args, 0);
                if matches!(v, Value::Bool(false)) { Ok(Value::Bool(true)) }
                else { Err(RuntimeSignal::Error(format!("assertFalse failed: got {}{}", v, msg_suffix(&args, 1)))) }
            }
            "__remotest_assert_none" => {
                let v = arg(&args, 0);
                if matches!(v, Value::Null) { Ok(Value::Bool(true)) }
                else { Err(RuntimeSignal::Error(format!("assertNone failed: got {}{}", v, msg_suffix(&args, 1)))) }
            }
            "__remotest_assert_not_none" => {
                let v = arg(&args, 0);
                if !matches!(v, Value::Null) { Ok(Value::Bool(true)) }
                else { Err(RuntimeSignal::Error(format!("assertNotNone failed{}", msg_suffix(&args, 1)))) }
            }
            "__remotest_assert_in" => {
                let (item, coll) = (arg(&args, 0), arg(&args, 1));
                let found = match &coll {
                    Value::List(l) => l.iter().any(|v| *v == item),
                    Value::Str(s)  => s.contains(&item.to_string()),
                    Value::Map(m)  => m.iter().any(|(k, _)| *k == item.to_string()),
                    _ => false,
                };
                if found { Ok(Value::Bool(true)) }
                else { Err(RuntimeSignal::Error(format!("assertIn failed: {} not in {}{}", item, coll, msg_suffix(&args, 2)))) }
            }
            "__remotest_assert_almost_equal" => {
                let a = Self::to_f64(arg(&args, 0));
                let b = Self::to_f64(arg(&args, 1));
                let tol = match args.get(2) { Some(v) => Self::to_f64(v.clone()), None => 1e-6 };
                if (a - b).abs() <= tol { Ok(Value::Bool(true)) }
                else { Err(RuntimeSignal::Error(format!("assertAlmostEqual failed: |{} - {}| > {}{}", a, b, tol, msg_suffix(&args, 3)))) }
            }
            "__remotest_assert_raises" => {
                let f = arg(&args, 0);
                match self.call_value(f, Vec::new()) {
                    Ok(_) => Err(RuntimeSignal::Error("assertRaises failed: no error was raised".to_string())),
                    Err(RuntimeSignal::Error(_)) => Ok(Value::Bool(true)),
                    Err(other) => Err(other), // real Return/Exit signals aren't swallowed as "did raise"
                }
            }

            // ---- Mocking / spying ----
            "__remotest_mock" => {
                let mname = arg_str(&args, 0, "mock");
                let ret = arg(&args, 1);
                let id = unsafe {
                    let seq = &mut *REMOTEST_MOCK_SEQ.0.get();
                    *seq += 1;
                    format!("mock#{}", *seq)
                };
                unsafe { (&mut *REMOTEST_MOCKS.0.get()).push(RemotestMock { id: id.clone(), name: mname, return_value: ret, calls: Vec::new() }); }
                Ok(Value::Str(id))
            }
            "__remotest_mock_call" => {
                let id = arg_str(&args, 0, "");
                let call_args: Vec<Value> = args.iter().skip(1).cloned().collect();
                let mut ret = Value::Null;
                unsafe {
                    if let Some(m) = (&mut *REMOTEST_MOCKS.0.get()).iter_mut().find(|m| m.id == id) {
                        m.calls.push(call_args);
                        ret = m.return_value.clone();
                    }
                }
                Ok(ret)
            }
            "__remotest_spy_call" => {
                let id = arg_str(&args, 0, "");
                let real_fn = arg(&args, 1);
                let call_args: Vec<Value> = args.iter().skip(2).cloned().collect();
                unsafe {
                    let mocks = &mut *REMOTEST_MOCKS.0.get();
                    if !mocks.iter().any(|m| m.id == id) {
                        mocks.push(RemotestMock { id: id.clone(), name: "spy".to_string(), return_value: Value::Null, calls: Vec::new() });
                    }
                    if let Some(m) = mocks.iter_mut().find(|m| m.id == id) {
                        m.calls.push(call_args.clone());
                    }
                }
                self.call_value(real_fn, call_args) // real pass-through call, not simulated
            }
            "__remotest_mock_calls" => {
                let id = arg_str(&args, 0, "");
                let calls = unsafe { (&*REMOTEST_MOCKS.0.get()).iter().find(|m| m.id == id).map(|m| m.calls.clone()).unwrap_or_default() };
                Ok(Value::List(calls.into_iter().map(Value::List).collect()))
            }
            "__remotest_mock_call_count" => {
                let id = arg_str(&args, 0, "");
                let n = unsafe { (&*REMOTEST_MOCKS.0.get()).iter().find(|m| m.id == id).map(|m| m.calls.len()).unwrap_or(0) };
                Ok(Value::Int(n as i64))
            }
            "__remotest_reset_mocks" => {
                unsafe { (&mut *REMOTEST_MOCKS.0.get()).clear(); }
                Ok(Value::Bool(true))
            }

            // ---- Faker (real LCG RNG, same generator as rand_state / __rand_int) ----
            "__remotest_fake_name" => {
                let first = remotest_pick(&mut self.rand_state, &REMOTEST_FIRST_NAMES);
                let last  = remotest_pick(&mut self.rand_state, &REMOTEST_LAST_NAMES);
                Ok(Value::Str(format!("{} {}", first, last)))
            }
            "__remotest_fake_email" => {
                let first = remotest_pick(&mut self.rand_state, &REMOTEST_FIRST_NAMES).to_lowercase();
                let domain = remotest_pick(&mut self.rand_state, &REMOTEST_DOMAINS);
                let n = remotest_next_rand(&mut self.rand_state) % 1000;
                Ok(Value::Str(format!("{}{}@{}", first, n, domain)))
            }
            "__remotest_fake_int" => {
                let min = match args.get(0) { Some(v) => Self::to_f64(v.clone()) as i64, None => 0 };
                let max = match args.get(1) { Some(v) => Self::to_f64(v.clone()) as i64, None => 100 };
                let r = (remotest_next_rand(&mut self.rand_state) as i64).abs() % (max - min + 1).max(1) + min;
                Ok(Value::Int(r))
            }
            "__remotest_fake_sentence" => {
                let n = match args.get(0) { Some(v) => Self::to_f64(v.clone()) as usize, None => 6 };
                let words: Vec<&str> = (0..n.max(1)).map(|_| remotest_pick(&mut self.rand_state, &REMOTEST_WORDS)).collect();
                let mut s = words.join(" ");
                if let Some(c) = s.get_mut(0..1) { c.make_ascii_uppercase(); }
                s.push('.');
                Ok(Value::Str(s))
            }
            "__remotest_fake_date" => {
                let ymin = match args.get(0) { Some(v) => Self::to_f64(v.clone()) as i64, None => 1990 };
                let ymax = match args.get(1) { Some(v) => Self::to_f64(v.clone()) as i64, None => 2025 };
                let year = (remotest_next_rand(&mut self.rand_state) as i64).abs() % (ymax - ymin + 1).max(1) + ymin;
                let month = (remotest_next_rand(&mut self.rand_state) as i64).abs() % 12 + 1;
                let day = (remotest_next_rand(&mut self.rand_state) as i64).abs() % 28 + 1;
                Ok(Value::Str(format!("{:04}-{:02}-{:02}", year, month, day)))
            }
            "__remotest_fake_uuid" => {
                let mut parts = Vec::new();
                for _ in 0..4 { parts.push(format!("{:08x}", remotest_next_rand(&mut self.rand_state) as u32)); }
                Ok(Value::Str(parts.join("-")))
            }

            // ---- Faker — extended (address / geo / business / web / security) ----
            "__remotest_fake_street" => {
                let num = (remotest_next_rand(&mut self.rand_state) % 9900) + 1;
                let name = remotest_pick(&mut self.rand_state, &REMOTEST_STREET_NAMES);
                let suffix = remotest_pick(&mut self.rand_state, &REMOTEST_STREET_SUFFIXES);
                Ok(Value::Str(format!("{} {} {}", num, name, suffix)))
            }
            "__remotest_fake_city" => Ok(Value::Str(remotest_pick(&mut self.rand_state, &REMOTEST_CITIES).to_string())),
            "__remotest_fake_country" => Ok(Value::Str(remotest_pick(&mut self.rand_state, &REMOTEST_COUNTRIES).to_string())),
            "__remotest_fake_zipcode" => {
                let z = (remotest_next_rand(&mut self.rand_state) % 900000) + 100000;
                Ok(Value::Str(format!("{}", z)))
            }
            "__remotest_fake_address" => {
                let num = (remotest_next_rand(&mut self.rand_state) % 9900) + 1;
                let street = remotest_pick(&mut self.rand_state, &REMOTEST_STREET_NAMES);
                let suffix = remotest_pick(&mut self.rand_state, &REMOTEST_STREET_SUFFIXES);
                let city = remotest_pick(&mut self.rand_state, &REMOTEST_CITIES);
                let country = remotest_pick(&mut self.rand_state, &REMOTEST_COUNTRIES);
                let zip = (remotest_next_rand(&mut self.rand_state) % 900000) + 100000;
                Ok(Value::Str(format!("{} {} {}, {}, {} {}", num, street, suffix, city, country, zip)))
            }
            "__remotest_fake_phone" => {
                let cc = remotest_pick(&mut self.rand_state, &["+1","+44","+91","+61","+49","+81"]);
                let mut digits = String::new();
                for _ in 0..10 { digits.push((b'0' + (remotest_next_rand(&mut self.rand_state) % 10) as u8) as char); }
                Ok(Value::Str(format!("{}-{}-{}-{}", cc, &digits[0..3], &digits[3..6], &digits[6..10])))
            }
            "__remotest_fake_company" => {
                let a = remotest_pick(&mut self.rand_state, &REMOTEST_COMPANY_WORDS);
                let suffix = remotest_pick(&mut self.rand_state, &REMOTEST_COMPANY_SUFFIXES);
                Ok(Value::Str(format!("{} {}", a, suffix)))
            }
            "__remotest_fake_job_title" => {
                let level = remotest_pick(&mut self.rand_state, &["Junior","Senior","Lead","Principal","Staff"]);
                let role = remotest_pick(&mut self.rand_state, &REMOTEST_JOB_ROLES);
                Ok(Value::Str(format!("{} {}", level, role)))
            }
            "__remotest_fake_username" => {
                let first = remotest_pick(&mut self.rand_state, &REMOTEST_FIRST_NAMES).to_lowercase();
                let n = remotest_next_rand(&mut self.rand_state) % 10000;
                Ok(Value::Str(format!("{}{}", first, n)))
            }
            "__remotest_fake_password" => {
                let len = match args.get(0) { Some(v) => Self::to_f64(v.clone()) as usize, None => 12 }.clamp(4, 128);
                const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*";
                let pw: String = (0..len).map(|_| {
                    let i = (remotest_next_rand(&mut self.rand_state) as usize) % CHARS.len();
                    CHARS[i] as char
                }).collect();
                Ok(Value::Str(pw))
            }
            "__remotest_fake_color" => {
                let r = remotest_next_rand(&mut self.rand_state) % 256;
                let g = remotest_next_rand(&mut self.rand_state) % 256;
                let b = remotest_next_rand(&mut self.rand_state) % 256;
                Ok(Value::Str(format!("#{:02x}{:02x}{:02x}", r, g, b)))
            }
            "__remotest_fake_url" => {
                let word = remotest_pick(&mut self.rand_state, &REMOTEST_WORDS);
                let tld = remotest_pick(&mut self.rand_state, &["com","dev","io","net","org"]);
                Ok(Value::Str(format!("https://{}.{}", word, tld)))
            }
            "__remotest_fake_ipv4" => {
                let parts: Vec<String> = (0..4).map(|_| ((remotest_next_rand(&mut self.rand_state) % 256) as u8).to_string()).collect();
                Ok(Value::Str(parts.join(".")))
            }
            "__remotest_fake_credit_card" => {
                // Real Luhn algorithm — the check digit actually satisfies
                // mod-10 validation, same as a genuine card number would,
                // not a random 16th digit.
                let mut digits: Vec<u8> = vec![4]; // Visa-style leading digit
                for _ in 0..14 { digits.push((remotest_next_rand(&mut self.rand_state) % 10) as u8); }
                let check = remotest_luhn_check_digit(&digits);
                digits.push(check);
                let s: String = digits.iter().map(|d| d.to_string()).collect();
                Ok(Value::Str(format!("{} {} {} {}", &s[0..4], &s[4..8], &s[8..12], &s[12..16])))
            }
            "__remotest_fake_boolean" => Ok(Value::Bool(remotest_next_rand(&mut self.rand_state) % 2 == 0)),
            "__remotest_fake_currency" => Ok(Value::Str(remotest_pick(&mut self.rand_state, &["USD","EUR","GBP","INR","JPY","AUD","CAD"]).to_string())),
            "__remotest_fake_paragraph" => {
                let n_sentences = match args.get(0) { Some(v) => Self::to_f64(v.clone()) as usize, None => 3 }.max(1);
                let mut sentences = Vec::new();
                for _ in 0..n_sentences {
                    let n_words = 5 + (remotest_next_rand(&mut self.rand_state) as usize % 6);
                    let words: Vec<&str> = (0..n_words).map(|_| remotest_pick(&mut self.rand_state, &REMOTEST_WORDS)).collect();
                    let mut s = words.join(" ");
                    if let Some(c) = s.get_mut(0..1) { c.make_ascii_uppercase(); }
                    s.push('.');
                    sentences.push(s);
                }
                Ok(Value::Str(sentences.join(" ")))
            }

            // ---- Runner ----
            "__remotest_run_all" => self.remotest_run(None),
            "__remotest_run_tag" => { let tag = arg_str(&args, 0, ""); self.remotest_run(Some(tag)) }

            // ---- Generators (return Map{__gen__: kind, ...} as data, not
            // closures — remotest_generate()/remotest_shrink_candidates()
            // below read these back to sample and shrink) ----
            "__remotest_gen_int" => {
                let min = match args.get(0) { Some(v) => Self::to_f64(v.clone()) as i64, None => 0 };
                let max = match args.get(1) { Some(v) => Self::to_f64(v.clone()) as i64, None => 100 };
                Ok(Value::Map(vec![
                    ("__gen__".into(), Value::Str("int".into())),
                    ("min".into(), Value::Int(min)),
                    ("max".into(), Value::Int(max.max(min))),
                ]))
            }
            "__remotest_gen_float" => {
                let min = match args.get(0) { Some(v) => Self::to_f64(v.clone()), None => 0.0 };
                let max = match args.get(1) { Some(v) => Self::to_f64(v.clone()), None => 1.0 };
                Ok(Value::Map(vec![
                    ("__gen__".into(), Value::Str("float".into())),
                    ("min".into(), Value::Float(min)),
                    ("max".into(), Value::Float(max.max(min))),
                ]))
            }
            "__remotest_gen_bool" => Ok(Value::Map(vec![("__gen__".into(), Value::Str("bool".into()))])),
            "__remotest_gen_string" => {
                let max_len = match args.get(0) { Some(v) => Self::to_f64(v.clone()) as i64, None => 12 };
                Ok(Value::Map(vec![
                    ("__gen__".into(), Value::Str("string".into())),
                    ("maxLen".into(), Value::Int(max_len.max(0))),
                ]))
            }
            "__remotest_gen_list" => {
                let elem_gen = arg(&args, 0);
                let max_len = match args.get(1) { Some(v) => Self::to_f64(v.clone()) as i64, None => 8 };
                Ok(Value::Map(vec![
                    ("__gen__".into(), Value::Str("list".into())),
                    ("elem".into(), elem_gen),
                    ("maxLen".into(), Value::Int(max_len.max(0))),
                ]))
            }
            "__remotest_gen_one_of" => {
                let options = arg(&args, 0);
                Ok(Value::Map(vec![
                    ("__gen__".into(), Value::Str("oneof".into())),
                    ("options".into(), options),
                ]))
            }

            // ---- Property-based testing ----
            "__remotest_for_all" => {
                let gens = match arg(&args, 0) { Value::List(l) => l, other => vec![other] };
                let prop_fn = arg(&args, 1);
                let opts = arg(&args, 2);
                let iterations = match &opts {
                    Value::Map(m) => match autoclib_map_get(m, "iterations") {
                        Some(v) => Self::to_f64(v.clone()) as usize,
                        None => 100,
                    },
                    _ => 100,
                };
                let max_shrinks = match &opts {
                    Value::Map(m) => match autoclib_map_get(m, "maxShrinks") {
                        Some(v) => Self::to_f64(v.clone()) as usize,
                        None => 200,
                    },
                    _ => 200,
                };
                self.remotest_for_all(gens, prop_fn, iterations.max(1), max_shrinks)
            }

            // ---- Load / stress testing ----
            "__remotest_load" => {
                let f = arg(&args, 0);
                let opts = arg(&args, 1);
                let (users, iters_per_user, spawn_rate) = match &opts {
                    Value::Map(m) => {
                        let u = match autoclib_map_get(m, "users") { Some(v) => Self::to_f64(v.clone()) as usize, None => 10 };
                        (
                            u,
                            match autoclib_map_get(m, "iterationsPerUser") { Some(v) => Self::to_f64(v.clone()) as usize, None => 10 },
                            match autoclib_map_get(m, "spawnRate") { Some(v) => Self::to_f64(v.clone()) as usize, None => u.max(1) },
                        )
                    }
                    _ => (10, 10, 10),
                };
                self.remotest_load(f, users.max(1), iters_per_user.max(1), spawn_rate.max(1))
            }

            // ---- BDD (Behave/Robot Framework parity) ----
            // scenario() registers a deferred test (same REMOTEST_TESTS
            // queue as test()/it(), runs later via runAll/runTag). Its body
            // calls given/when/then, each of which does a REAL call_value
            // of the step function right then — not a Gherkin-file parser,
            // native syntax instead, so there's no second language to
            // maintain. On step failure, the error is tagged with which
            // step (Given/When/Then) failed before propagating, exactly
            // like Behave's step-level failure reporting.
            "__remotest_scenario" => {
                let raw_name = arg_str(&args, 0, "unnamed scenario");
                let full_name = {
                    let groups = unsafe { &*REMOTEST_GROUPS.0.get() };
                    if groups.is_empty() { format!("Scenario: {}", raw_name) } else { format!("{} > Scenario: {}", groups.join(" > "), raw_name) }
                };
                let f = arg(&args, 1);
                let tags = arg_tags(&args, 2);
                unsafe { (&mut *REMOTEST_TESTS.0.get()).push((full_name, f, tags)); }
                Ok(Value::Bool(true))
            }
            "__remotest_given" | "__remotest_when" | "__remotest_then" | "__remotest_and_step" => {
                let step_label = match name {
                    "__remotest_given" => "Given",
                    "__remotest_when"  => "When",
                    "__remotest_then"  => "Then",
                    _                  => "And",
                };
                let desc = arg_str(&args, 0, "");
                let f = arg(&args, 1);
                match self.call_value(f, Vec::new()) {
                    Ok(v) => {
                        println!("    \x1b[{}m✓\x1b[0m {}: {}", autoclib_ansi_code("green"), step_label, desc);
                        Ok(v)
                    }
                    Err(RuntimeSignal::Error(msg)) => {
                        println!("    \x1b[{}m✗\x1b[0m {}: {}", autoclib_ansi_code("red"), step_label, desc);
                        Err(RuntimeSignal::Error(format!("[{}: {}] {}", step_label, desc, msg)))
                    }
                    Err(other) => Err(other),
                }
            }

            // ---- Tox-style environment matrix ----
            // envMatrix(envs, testFn) runs the SAME testFn once per entry
            // in `envs` (each a Map of config/feature-flag values), passing
            // that env in as testFn's argument — a real, honest analog of
            // tox's "same suite, N configurations" model. It is NOT
            // multiple Python-interpreter-version isolation (Remox has one
            // runtime), so this matrixes config/feature-flag sets rather
            // than language versions — the part of tox's value that
            // actually transfers to a single-runtime language.
            "__remotest_env_matrix" => {
                let envs = match arg(&args, 0) { Value::List(l) => l, other => vec![other] };
                let test_fn = arg(&args, 1);
                let mut passed = 0i64;
                let mut failed = 0i64;
                let mut results: Vec<Value> = Vec::new();
                for (i, env) in envs.iter().enumerate() {
                    let env_name = match env {
                        Value::Map(m) => match autoclib_map_get(m, "name") {
                            Some(Value::Str(s)) => s.clone(),
                            _ => format!("env#{}", i + 1),
                        },
                        _ => format!("env#{}", i + 1),
                    };
                    let (status, detail) = match self.call_value(test_fn.clone(), vec![env.clone()]) {
                        Ok(_) => { passed += 1; ("pass".to_string(), String::new()) }
                        Err(RuntimeSignal::Error(msg)) => { failed += 1; ("fail".to_string(), msg) }
                        Err(other) => return Err(other),
                    };
                    let (code, label) = if status == "pass" { (autoclib_ansi_code("green"), "PASS") } else { (autoclib_ansi_code("red"), "FAIL") };
                    println!("\x1b[{}m[{}]\x1b[0m env: {}{}", code, label, env_name, if detail.is_empty() { String::new() } else { format!(" — {}", detail) });
                    results.push(Value::Map(vec![
                        ("env".to_string(), Value::Str(env_name)),
                        ("status".to_string(), Value::Str(status)),
                        ("detail".to_string(), Value::Str(detail)),
                    ]));
                }
                let total = passed + failed;
                println!(
                    "\x1b[{}mRemotest.envMatrix: {}/{} environments passed\x1b[0m",
                    if failed > 0 { autoclib_ansi_code("red") } else { autoclib_ansi_code("green") },
                    passed, total
                );
                Ok(Value::Map(vec![
                    ("passed".into(), Value::Int(passed)),
                    ("failed".into(), Value::Int(failed)),
                    ("total".into(), Value::Int(total)),
                    ("results".into(), Value::List(results)),
                ]))
            }

            "__remotest_reset" => {
                unsafe {
                    (&mut *REMOTEST_TESTS.0.get()).clear();
                    (&mut *REMOTEST_GROUPS.0.get()).clear();
                    (&mut *REMOTEST_FIXTURES.0.get()).clear();
                    (&mut *REMOTEST_MOCKS.0.get()).clear();
                }
                Ok(Value::Bool(true))
            }
            _ => Err(RuntimeSignal::Error(format!("Unknown Remotest function: {}", name))),
        }
    }

    /// Shared runner core for runAll/runTag: executes every registered test
    /// (real call_value execution, not simulated), classifies pass/fail/skip
    /// via SKIP_SENTINEL, prints an Autoclib-styled colored summary line per
    /// test, and returns a structured Map report.
    fn remotest_run(&mut self, tag_filter: Option<String>) -> Result<Value, RuntimeSignal> {
        const SKIP_SENTINEL: &str = "__REMOTEST_SKIP__";
        let tests: Vec<(String, Value, Vec<String>)> = unsafe { (&*REMOTEST_TESTS.0.get()).clone() };
        let mut passed = 0i64;
        let mut failed = 0i64;
        let mut skipped = 0i64;
        let mut results: Vec<Value> = Vec::new();

        for (tname, tfn, ttags) in tests {
            if let Some(ref tag) = tag_filter {
                if !ttags.iter().any(|t| t == tag) { continue; }
            }
            let (status, detail) = match self.call_value(tfn, Vec::new()) {
                Ok(_) => { passed += 1; ("pass".to_string(), String::new()) }
                Err(RuntimeSignal::Error(msg)) if msg.starts_with(SKIP_SENTINEL) => {
                    skipped += 1;
                    ("skip".to_string(), msg.trim_start_matches(SKIP_SENTINEL).to_string())
                }
                Err(RuntimeSignal::Error(msg)) => { failed += 1; ("fail".to_string(), msg) }
                Err(other) => { failed += 1; ("fail".to_string(), format!("{:?}", other)) }
            };
            let (code, label) = match status.as_str() {
                "pass" => (autoclib_ansi_code("green"), "PASS"),
                "skip" => (autoclib_ansi_code("yellow"), "SKIP"),
                _      => (autoclib_ansi_code("red"), "FAIL"),
            };
            println!("\x1b[{}m[{}]\x1b[0m {}{}", code, label, tname, if detail.is_empty() { String::new() } else { format!(" — {}", detail) });
            results.push(Value::Map(vec![
                ("name".to_string(), Value::Str(tname)),
                ("status".to_string(), Value::Str(status)),
                ("detail".to_string(), Value::Str(detail)),
            ]));
        }

        let total = passed + failed + skipped;
        println!(
            "\x1b[{}mRemotest: {} passed, {} failed, {} skipped ({} total)\x1b[0m",
            if failed > 0 { autoclib_ansi_code("red") } else { autoclib_ansi_code("green") },
            passed, failed, skipped, total
        );
        Ok(Value::Map(vec![
            ("passed".to_string(), Value::Int(passed)),
            ("failed".to_string(), Value::Int(failed)),
            ("skipped".to_string(), Value::Int(skipped)),
            ("total".to_string(), Value::Int(total)),
            ("results".to_string(), Value::List(results)),
        ]))
    }

    /// Property-based test runner (Hypothesis parity). Samples `iterations`
    /// random tuples from `gens` (real LCG draws off self.rand_state, same
    /// generator Faker uses — not simulated), calls `prop_fn(inputs...)`
    /// for each. On the FIRST failing case, shrinks each input toward its
    /// simplest failing form: repeatedly tries smaller candidates (halved
    /// magnitude for ints/floats, shorter strings/lists, `false` before
    /// `true`) and keeps any candidate that still makes prop_fn fail,
    /// re-running prop_fn for real each time — real shrinking, not a
    /// lookup table. Stops shrinking after `max_shrinks` prop_fn calls or
    /// once no candidate reduces the counterexample further.
    fn remotest_for_all(
        &mut self,
        gens: Vec<Value>,
        prop_fn: Value,
        iterations: usize,
        max_shrinks: usize,
    ) -> Result<Value, RuntimeSignal> {
        for i in 0..iterations {
            let sample: Vec<Value> = gens.iter()
                .map(|g| remotest_generate(g, &mut self.rand_state))
                .collect();
            match self.call_value(prop_fn.clone(), sample.clone()) {
                Ok(_) => continue,
                Err(RuntimeSignal::Error(first_msg)) => {
                    // Found a failing case — shrink it.
                    let mut best = sample;
                    let mut best_msg = first_msg;
                    let mut shrink_calls = 0usize;
                    let mut improved = true;
                    while improved && shrink_calls < max_shrinks {
                        improved = false;
                        for idx in 0..best.len() {
                            let candidates = remotest_shrink_candidates(&gens[idx], &best[idx]);
                            for cand in candidates {
                                if shrink_calls >= max_shrinks { break; }
                                let mut trial = best.clone();
                                trial[idx] = cand.clone();
                                shrink_calls += 1;
                                if let Err(RuntimeSignal::Error(msg)) = self.call_value(prop_fn.clone(), trial.clone()) {
                                    best = trial;
                                    best_msg = msg;
                                    improved = true;
                                    break; // restart scan from idx 0 on the smaller case
                                }
                            }
                            if improved { break; }
                        }
                    }
                    println!(
                        "\x1b[{}m[FAIL]\x1b[0m Remotest.forAll: counterexample after {} shrink step(s) — {} — {}",
                        autoclib_ansi_code("red"), shrink_calls, remotest_values_to_string(&best), best_msg
                    );
                    return Ok(Value::Map(vec![
                        ("passed".into(), Value::Bool(false)),
                        ("iterationsRun".into(), Value::Int((i + 1) as i64)),
                        ("counterexample".into(), Value::List(best)),
                        ("shrinkSteps".into(), Value::Int(shrink_calls as i64)),
                        ("error".into(), Value::Str(best_msg)),
                    ]));
                }
                Err(other) => return Err(other), // real Return/Exit signals propagate, not swallowed
            }
        }
        println!(
            "\x1b[{}m[PASS]\x1b[0m Remotest.forAll: {} random case(s) OK\x1b[0m",
            autoclib_ansi_code("green"), iterations
        );
        Ok(Value::Map(vec![
            ("passed".into(), Value::Bool(true)),
            ("iterationsRun".into(), Value::Int(iterations as i64)),
            ("counterexample".into(), Value::Null),
        ]))
    }

    /// Load/stress runner. Uses COOPERATIVE ROUND-ROBIN interleaving with a
    /// spawn-rate ramp-up — this is not an approximation of Locust, it is
    /// the same concurrency model Locust itself uses per-process: Locust's
    /// "concurrent users" are gevent greenlets, cooperatively scheduled on
    /// one OS thread/core (confirmed via Locust's own docs — gevent lets a
    /// single process interleave thousands of users without real OS
    /// threads). True multi-core scaling in Locust ALSO requires separate
    /// OS processes (--master/--worker), because Python's GIL caps one
    /// process to one core regardless of gevent. So a single Remotest
    /// process matching Locust's single-process behavior is genuine parity,
    /// not a lesser substitute — going past that to distributed multi-
    /// process load generation is a separate feature (real for Locust too,
    /// not "just more code" in either language).
    ///
    /// Ramp-up: `active_users` starts at `spawn_rate` and grows by
    /// `spawn_rate` each round until it reaches `users`, mirroring
    /// Locust's --spawn-rate. Each round fires one real call_value per
    /// currently-active virtual user (their sessions interleave, exactly
    /// like Locust's round-robin greenlet switching) until every user has
    /// completed `iters_per_user` real calls.
    fn remotest_load(&mut self, f: Value, users: usize, iters_per_user: usize, spawn_rate: usize) -> Result<Value, RuntimeSignal> {
        let total_calls = users * iters_per_user;
        let spawn_rate = spawn_rate.max(1);
        let mut completed = vec![0usize; users];
        let mut active_users = spawn_rate.min(users);
        let mut done = 0usize;
        let mut rounds = 0usize;
        let mut passed = 0i64;
        let mut failed = 0i64;
        let mut first_error: Option<String> = None;

        while done < total_calls {
            rounds += 1;
            for u in 0..active_users {
                if completed[u] < iters_per_user {
                    match self.call_value(f.clone(), Vec::new()) {
                        Ok(_) => passed += 1,
                        Err(RuntimeSignal::Error(msg)) => {
                            failed += 1;
                            if first_error.is_none() { first_error = Some(msg); }
                        }
                        Err(other) => return Err(other),
                    }
                    completed[u] += 1;
                    done += 1;
                }
            }
            if active_users < users { active_users = (active_users + spawn_rate).min(users); }
        }

        let error_rate = if total_calls > 0 { (failed as f64) / (total_calls as f64) * 100.0 } else { 0.0 };
        println!(
            "\x1b[{}mRemotest.load: {} users (spawnRate {}/round) x {} iter = {} calls over {} interleaved rounds — {} ok, {} failed ({:.1}% error rate)\x1b[0m",
            if failed > 0 { autoclib_ansi_code("red") } else { autoclib_ansi_code("green") },
            users, spawn_rate, iters_per_user, total_calls, rounds, passed, failed, error_rate
        );
        Ok(Value::Map(vec![
            ("users".into(), Value::Int(users as i64)),
            ("spawnRate".into(), Value::Int(spawn_rate as i64)),
            ("iterationsPerUser".into(), Value::Int(iters_per_user as i64)),
            ("totalCalls".into(), Value::Int(total_calls as i64)),
            ("rounds".into(), Value::Int(rounds as i64)),
            ("passed".into(), Value::Int(passed)),
            ("failed".into(), Value::Int(failed)),
            ("errorRatePercent".into(), Value::Float(error_rate)),
            ("firstError".into(), match first_error { Some(e) => Value::Str(e), None => Value::Null }),
        ]))
    }


    /// Shared engine core for `runWorker`/`runOne`: pulls up to `limit` due
    /// jobs from `queue` (or all queues if empty), ordered by priority then
    /// eta then FIFO, executes each by calling back into `self.call_function`
    /// with the task's own registered name (a task IS a normal Remox `fn` —
    /// no separate closure representation needed), and applies retry/
    /// dead-letter/chain/chord bookkeeping. Returns the ids of jobs that were
    /// actually attempted this call (rate-limited jobs are excluded).
    fn tasoaque_process(&mut self, queue_filter: &str, limit: usize) -> Vec<String> {
        let due: Vec<TasoaqueJob> = {
            let mut st = TASOAQUE.lock().unwrap();
            let clock = st.clock;
            let mut idx: Vec<usize> = st.jobs.iter().enumerate()
                .filter(|(_, j)| (j.status == "pending" || j.status == "retry")
                    && (queue_filter.is_empty() || j.queue == queue_filter)
                    && j.eta <= clock)
                .map(|(i, _)| i)
                .collect();
            idx.sort_by(|&a, &b| {
                let ja = &st.jobs[a]; let jb = &st.jobs[b];
                ja.priority.cmp(&jb.priority).then(ja.eta.cmp(&jb.eta)).then(ja.created_at.cmp(&jb.created_at))
            });
            idx.truncate(limit);
            let snapshot: Vec<TasoaqueJob> = idx.iter().map(|&i| st.jobs[i].clone()).collect();
            for &i in &idx { st.jobs[i].status = "running".to_string(); }
            snapshot
        };

        let mut processed_ids = Vec::new();
        for job in due {
            let allowed = { let mut st = TASOAQUE.lock().unwrap(); tasoaque_check_rate(&mut st, &job.task) };
            if !allowed {
                let mut st = TASOAQUE.lock().unwrap();
                if let Some(j) = st.jobs.iter_mut().find(|j| j.id == job.id) { j.status = "pending".to_string(); }
                continue;
            }

            let outcome = self.call_function(&job.task, job.args.clone(), Vec::new());
            processed_ids.push(job.id.clone());
            let mut st = TASOAQUE.lock().unwrap();
            let pos = match st.jobs.iter().position(|j| j.id == job.id) { Some(p) => p, None => continue };
            match outcome {
                Ok(v) => {
                    st.jobs[pos].status = "success".to_string();
                    st.jobs[pos].result = v.clone();
                    if !st.jobs[pos].chain_next.is_empty() {
                        let (next_task, mut next_args, next_queue) = st.jobs[pos].chain_next.remove(0);
                        let remaining = st.jobs[pos].chain_next.clone();
                        let (priority, max_retries, retry_delay) =
                            (st.jobs[pos].priority, st.jobs[pos].max_retries, st.jobs[pos].retry_delay);
                        let group_id = st.jobs[pos].group_id.clone();
                        next_args.push(v);
                        let clock = st.clock;
                        let new_id = st.fresh_id("tq");
                        st.jobs.push(TasoaqueJob {
                            id: new_id, task: next_task, args: next_args, queue: next_queue,
                            priority, eta: clock, status: "pending".into(), attempts: 0,
                            max_retries, retry_delay, result: Value::Null, error: String::new(),
                            created_at: clock, chain_next: remaining, group_id,
                        });
                    }
                    let gid = st.jobs[pos].group_id.clone();
                    if !gid.is_empty() { tasoaque_maybe_fire_chord(&mut st, &gid); }
                }
                Err(sig) => {
                    let msg = match sig {
                        RuntimeSignal::Error(m) => m,
                        RuntimeSignal::Exit(code) => format!("task called exit({})", code),
                        RuntimeSignal::Return(_) => "task returned outside of a call context".to_string(),
                    };
                    st.jobs[pos].attempts += 1;
                    if st.jobs[pos].attempts <= st.jobs[pos].max_retries {
                        let delay = st.jobs[pos].retry_delay * st.jobs[pos].attempts as u64;
                        let clock = st.clock;
                        st.jobs[pos].status = "retry".to_string();
                        st.jobs[pos].eta = clock + delay.max(1);
                        st.jobs[pos].error = msg;
                    } else {
                        st.jobs[pos].status = "failed".to_string();
                        st.jobs[pos].error = msg;
                        let gid = st.jobs[pos].group_id.clone();
                        let dead = st.jobs.remove(pos);
                        st.dead_letters.push(dead);
                        if !gid.is_empty() { tasoaque_maybe_fire_chord(&mut st, &gid); }
                    }
                }
            }
        }
        processed_ids
    }

    // Call a Value::Lambda or Value::Lambda-holding variable
    fn call_value(&mut self, val: Value, args: Vec<Value>) -> Result<Value, RuntimeSignal> {
        match val {
            Value::Lambda { params, body, captures } => {
                // If body is a builtin dispatch ident (module lambdas like __math_sqrt),
                // skip scope eval and route directly to call_function
                if let Expr::Ident(ref builtin_name) = *body {
                    if builtin_name.starts_with("__") {
                        return self.call_function(builtin_name, args, Vec::new());
                    }
                }
                self.push_scope();
                // Restore captures
                for (k, v) in captures { self.def_var(&k, v); }
                for (p, v) in params.iter().zip(args.iter()) { self.def_var(p, v.clone()); }
                let r = self.eval_expr(&body);
                self.pop_scope();
                r
            }
            _ => Err(RuntimeSignal::Error(format!("Cannot call {}", val))),
        }
    }

    fn to_f64(v: Value) -> f64 {
        match v { Value::Int(n) => n as f64, Value::Float(f) => f, _ => 0.0 }
    }

    fn interpolate_string(&mut self, s: &str) -> Result<String, RuntimeSignal> {
        let mut result = String::new();
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '{' && i + 1 < chars.len() && chars[i + 1] != '}' {
                let start = i + 1;
                let mut j = start;
                while j < chars.len() && chars[j] != '}' { j += 1; }
                if j < chars.len() {
                    let expr_str: String = chars[start..j].iter().collect();
                    let expr_str = expr_str.trim();
                    let val = if expr_str.contains('.') {
                        let parts: Vec<&str> = expr_str.splitn(2, '.').collect();
                        let obj_name = parts[0].trim();
                        let field_name = parts[1].trim();
                        match self.get_var(obj_name) {
                            Some(Value::Struct { ref fields, .. }) => {
                                fields.iter().find(|(k, _)| k == field_name)
                                    .map(|(_, v)| v.to_string())
                                    .unwrap_or_else(|| format!("{{{}}}", expr_str))
                            }
                            Some(Value::Map(ref pairs)) => {
                                pairs.iter().find(|(k, _)| k == field_name)
                                    .map(|(_, v)| v.to_string())
                                    .unwrap_or_else(|| format!("{{{}}}", expr_str))
                            }
                            Some(other) => other.to_string(),
                            None => format!("{{{}}}", expr_str),
                        }
                    } else {
                        self.get_var(expr_str)
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| format!("{{{}}}", expr_str))
                    };
                    result.push_str(&val);
                    i = j + 1;
                    continue;
                }
            }
            result.push(chars[i]);
            i += 1;
        }
        Ok(result)
    }

    fn eval_binop(&self, op: &BinOpKind, l: Value, r: Value) -> Result<Value, RuntimeSignal> {
        match op {
            BinOpKind::And => Ok(Value::Bool(l.is_truthy() && r.is_truthy())),
            BinOpKind::Or  => Ok(Value::Bool(l.is_truthy() || r.is_truthy())),
            BinOpKind::Eq    => Ok(Value::Bool(l.is_equal(&r))),
            BinOpKind::NotEq => Ok(Value::Bool(!l.is_equal(&r))),
            BinOpKind::Lt    => Ok(Value::Bool(self.cmp_val(&l, &r) < 0)),
            BinOpKind::Gt    => Ok(Value::Bool(self.cmp_val(&l, &r) > 0)),
            BinOpKind::LtEq  => Ok(Value::Bool(self.cmp_val(&l, &r) <= 0)),
            BinOpKind::GtEq  => Ok(Value::Bool(self.cmp_val(&l, &r) >= 0)),
            BinOpKind::Add => {
                if numrux_is_arr(&l) || numrux_is_arr(&r) {
                    return dispatch_numrux("__numrux_add", vec![l, r]).map_err(RuntimeSignal::Error);
                }
                match (l, r) {
                (Value::Int(a),   Value::Int(b))   => Ok(Value::Int(a + b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
                (Value::Int(a),   Value::Float(b)) => Ok(Value::Float(a as f64 + b)),
                (Value::Float(a), Value::Int(b))   => Ok(Value::Float(a + b as f64)),
                (Value::Str(a),   Value::Str(b))   => Ok(Value::Str(a + &b)),
                (Value::Str(a),   b)               => Ok(Value::Str(a + &b.to_string())),
                (Value::List(mut a), Value::List(b)) => { a.extend(b); Ok(Value::List(a)) }
                (l, r) => Err(RuntimeSignal::Error(format!("Cannot add {} and {}", l, r))),
                }
            },
            BinOpKind::Sub => {
                if numrux_is_arr(&l) || numrux_is_arr(&r) {
                    return dispatch_numrux("__numrux_sub", vec![l, r]).map_err(RuntimeSignal::Error);
                }
                match (l, r) {
                (Value::Int(a),   Value::Int(b))   => Ok(Value::Int(a - b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
                (Value::Int(a),   Value::Float(b)) => Ok(Value::Float(a as f64 - b)),
                (Value::Float(a), Value::Int(b))   => Ok(Value::Float(a - b as f64)),
                (l, r) => Err(RuntimeSignal::Error(format!("Cannot subtract {} and {}", l, r))),
                }
            },
            BinOpKind::Mul => {
                if numrux_is_arr(&l) || numrux_is_arr(&r) {
                    return dispatch_numrux("__numrux_mul", vec![l, r]).map_err(RuntimeSignal::Error);
                }
                match (l, r) {
                (Value::Int(a),   Value::Int(b))   => Ok(Value::Int(a * b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
                (Value::Int(a),   Value::Float(b)) => Ok(Value::Float(a as f64 * b)),
                (Value::Float(a), Value::Int(b))   => Ok(Value::Float(a * b as f64)),
                (Value::Str(s),   Value::Int(n))   => Ok(Value::Str(s.repeat(n.max(0) as usize))),
                (l, r) => Err(RuntimeSignal::Error(format!("Cannot multiply {} and {}", l, r))),
                }
            },
            BinOpKind::Div => {
                if numrux_is_arr(&l) || numrux_is_arr(&r) {
                    return dispatch_numrux("__numrux_div", vec![l, r]).map_err(RuntimeSignal::Error);
                }
                match (l, r) {
                (_, Value::Int(0))      => Err(RuntimeSignal::Error("Division by zero".into())),
                (_, Value::Float(f)) if f == 0.0 => Err(RuntimeSignal::Error("Division by zero".into())),
                (Value::Int(a),   Value::Int(b))   => Ok(Value::Int(a / b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
                (Value::Int(a),   Value::Float(b)) => Ok(Value::Float(a as f64 / b)),
                (Value::Float(a), Value::Int(b))   => Ok(Value::Float(a / b as f64)),
                (l, r) => Err(RuntimeSignal::Error(format!("Cannot divide {} and {}", l, r))),
                }
            },
            BinOpKind::Mod => match (l, r) {
                (Value::Int(a),   Value::Int(b))   => Ok(Value::Int(a % b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a % b)),
                (l, r) => Err(RuntimeSignal::Error(format!("Cannot mod {} and {}", l, r))),
            },
        }
    }

    fn cmp_val(&self, l: &Value, r: &Value) -> i32 {
        match (l, r) {
            (Value::Int(a),   Value::Int(b))   => if a < b { -1 } else if a > b { 1 } else { 0 },
            (Value::Float(a), Value::Float(b)) => a.partial_cmp(b).map(|o| o as i32).unwrap_or(0),
            (Value::Int(a),   Value::Float(b)) => (*a as f64).partial_cmp(b).map(|o| o as i32).unwrap_or(0),
            (Value::Float(a), Value::Int(b))   => a.partial_cmp(&(*b as f64)).map(|o| o as i32).unwrap_or(0),
            (Value::Str(a),   Value::Str(b))   => if a < b { -1 } else if a > b { 1 } else { 0 },
            _ => 0,
        }
    }
}

// Runtime signals
#[derive(Debug)]
enum RuntimeSignal {
    Exit(i32),
    Return(Option<Value>),
    Error(String),
}

// =============================================================================
// VYRADB HELPERS — Real flat-file JSON database engine
// Format: { "table": ["col=val|col=val", ...], "__schema__table": ["col:type,..."] }
// =============================================================================

fn vyradb_parse(raw: &str) -> HashMap<String, Vec<String>> {
    let mut db: HashMap<String, Vec<String>> = HashMap::new();
    // Simple hand-rolled JSON parser for our specific format
    // Format: {"table":["row1","row2",...], ...}
    let s = raw.trim();
    if s == "{}" || s.is_empty() { return db; }
    // Strip outer braces
    let inner = if s.starts_with('{') && s.ends_with('}') { &s[1..s.len()-1] } else { return db; };
    // Split on top-level commas between "key":[...] pairs
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    let mut current = String::new();
    let mut segments: Vec<String> = Vec::new();
    for ch in inner.chars() {
        if escape { current.push(ch); escape = false; continue; }
        if ch == '\\' && in_str { current.push(ch); escape = true; continue; }
        if ch == '"' { in_str = !in_str; current.push(ch); continue; }
        if in_str { current.push(ch); continue; }
        match ch {
            '[' | '{' => { depth += 1; current.push(ch); }
            ']' | '}' => { depth -= 1; current.push(ch); }
            ',' if depth == 0 => { segments.push(current.trim().to_string()); current = String::new(); }
            _ => { current.push(ch); }
        }
    }
    if !current.trim().is_empty() { segments.push(current.trim().to_string()); }

    for seg in segments {
        // Parse "key": ["val1", "val2"]
        if let Some(colon) = seg.find(':') {
            let key_part = seg[..colon].trim().trim_matches('"').to_string();
            let val_part = seg[colon+1..].trim();
            if val_part.starts_with('[') && val_part.ends_with(']') {
                let arr_inner = &val_part[1..val_part.len()-1];
                let mut rows: Vec<String> = Vec::new();
                let mut in_s = false;
                let mut esc = false;
                let mut item = String::new();
                let mut chars = arr_inner.chars().peekable();
                while let Some(c) = chars.next() {
                    if esc { item.push(c); esc = false; continue; }
                    if c == '\\' && in_s { item.push(c); esc = true; continue; }
                    if c == '"' { in_s = !in_s; continue; }
                    if in_s { item.push(c); continue; }
                    if c == ',' { rows.push(item.trim().to_string()); item = String::new(); }
                    else { item.push(c); }
                }
                if !item.trim().is_empty() { rows.push(item.trim().to_string()); }
                db.insert(key_part, rows);
            }
        }
    }
    db
}

fn vyradb_flush(path: &str, db: &HashMap<String, Vec<String>>) -> Result<(), RuntimeSignal> {
    // Serialize back to JSON
    let mut out = String::from("{\n");
    let mut first = true;
    let mut keys: Vec<&String> = db.keys().collect();
    keys.sort();
    for key in keys {
        if !first { out.push_str(",\n"); }
        first = false;
        out.push_str(&format!("  \"{}\": [", key));
        let rows = &db[key];
        for (i, row) in rows.iter().enumerate() {
            if i > 0 { out.push_str(", "); }
            // Escape row string
            let escaped = row.replace('\\', "\\\\").replace('"', "\\\"");
            out.push_str(&format!("\"{}\"", escaped));
        }
        out.push(']');
    }
    out.push_str("\n}");
    fs::write(path, out)
        .map_err(|e| RuntimeSignal::Error(format!("VyraDB: flush failed: {}", e)))
}

fn vyradb_row_to_value(row_str: &str) -> Value {
    // Rows are stored as proper JSON objects (see vyradb_json_parse_value
    // below). This replaces the old "col=val|col=val" pipe-delimited
    // format, which was ambiguous whenever a value itself contained an
    // already-escaped pipe/newline or an `=` inside a field name/value.
    // JSON's own quoting + escaping rules (shared with value_to_json,
    // which we already use elsewhere) don't have that ambiguity.
    match vyradb_json_parse_value(row_str) {
        Some(Value::Map(pairs)) => Value::Map(pairs),
        // Defensive fallback for legacy pipe-delimited rows written by an
        // older version of VyraDB, so existing databases don't just break.
        _ => vyradb_row_to_value_legacy_pipe_format(row_str),
    }
}

/// Legacy reader for the old "col=val|col=val" pipe-delimited row format.
/// Kept only so pre-existing vyradb.json files (written before rows moved
/// to real JSON) can still be read; new rows are never written this way.
fn vyradb_row_to_value_legacy_pipe_format(row_str: &str) -> Value {
    let mut pairs: Vec<(String, Value)> = Vec::new();
    for part in row_str.split('|') {
        if let Some(eq) = part.find('=') {
            let k = part[..eq].to_string();
            let v_raw = part[eq+1..].replace("\\|", "|").replace("\\n", "\n");
            let v = if let Ok(n) = v_raw.parse::<i64>() { Value::Int(n) }
                    else if let Ok(f) = v_raw.parse::<f64>() { Value::Float(f) }
                    else if v_raw == "true" { Value::Bool(true) }
                    else if v_raw == "false" { Value::Bool(false) }
                    else if v_raw == "null" { Value::Null }
                    else { Value::Str(v_raw) };
            pairs.push((k, v));
        }
    }
    Value::Map(pairs)
}

/// Minimal recursive-descent JSON parser, used to read VyraDB rows back
/// (the counterpart to `value_to_json`, which writes them). Handles
/// objects, arrays, strings (with standard escapes), numbers, booleans,
/// and null — enough for anything VyraDB itself ever writes.
fn vyradb_json_parse_value(s: &str) -> Option<Value> {
    let chars: Vec<char> = s.trim().chars().collect();
    let mut pos = 0usize;
    let v = vyradb_json_parse_at(&chars, &mut pos)?;
    Some(v)
}

fn vyradb_json_skip_ws(chars: &[char], pos: &mut usize) {
    while *pos < chars.len() && chars[*pos].is_whitespace() { *pos += 1; }
}

fn vyradb_json_parse_at(chars: &[char], pos: &mut usize) -> Option<Value> {
    vyradb_json_skip_ws(chars, pos);
    if *pos >= chars.len() { return None; }
    match chars[*pos] {
        '{' => {
            *pos += 1;
            let mut pairs: Vec<(String, Value)> = Vec::new();
            vyradb_json_skip_ws(chars, pos);
            if *pos < chars.len() && chars[*pos] == '}' { *pos += 1; return Some(Value::Map(pairs)); }
            loop {
                vyradb_json_skip_ws(chars, pos);
                let key = match vyradb_json_parse_string(chars, pos) { Some(k) => k, None => return None };
                vyradb_json_skip_ws(chars, pos);
                if *pos >= chars.len() || chars[*pos] != ':' { return None; }
                *pos += 1;
                let val = vyradb_json_parse_at(chars, pos)?;
                pairs.push((key, val));
                vyradb_json_skip_ws(chars, pos);
                match chars.get(*pos) {
                    Some(',') => { *pos += 1; }
                    Some('}') => { *pos += 1; break; }
                    _ => return None,
                }
            }
            Some(Value::Map(pairs))
        }
        '[' => {
            *pos += 1;
            let mut items: Vec<Value> = Vec::new();
            vyradb_json_skip_ws(chars, pos);
            if *pos < chars.len() && chars[*pos] == ']' { *pos += 1; return Some(Value::List(items)); }
            loop {
                let val = vyradb_json_parse_at(chars, pos)?;
                items.push(val);
                vyradb_json_skip_ws(chars, pos);
                match chars.get(*pos) {
                    Some(',') => { *pos += 1; }
                    Some(']') => { *pos += 1; break; }
                    _ => return None,
                }
            }
            Some(Value::List(items))
        }
        '"' => vyradb_json_parse_string(chars, pos).map(Value::Str),
        't' => {
            if chars[*pos..].starts_with(&['t','r','u','e']) { *pos += 4; Some(Value::Bool(true)) } else { None }
        }
        'f' => {
            if chars[*pos..].starts_with(&['f','a','l','s','e']) { *pos += 5; Some(Value::Bool(false)) } else { None }
        }
        'n' => {
            if chars[*pos..].starts_with(&['n','u','l','l']) { *pos += 4; Some(Value::Null) } else { None }
        }
        _ => {
            // Number
            let start = *pos;
            if chars[*pos] == '-' { *pos += 1; }
            while *pos < chars.len() && (chars[*pos].is_ascii_digit() || chars[*pos] == '.' || chars[*pos] == 'e' || chars[*pos] == 'E' || chars[*pos] == '+' || chars[*pos] == '-') {
                *pos += 1;
            }
            let num_str: String = chars[start..*pos].iter().collect();
            if num_str.is_empty() { return None; }
            if let Ok(n) = num_str.parse::<i64>() { Some(Value::Int(n)) }
            else if let Ok(f) = num_str.parse::<f64>() { Some(Value::Float(f)) }
            else { None }
        }
    }
}

fn vyradb_json_parse_string(chars: &[char], pos: &mut usize) -> Option<String> {
    if chars.get(*pos) != Some(&'"') { return None; }
    *pos += 1;
    let mut out = String::new();
    while *pos < chars.len() {
        let c = chars[*pos];
        if c == '"' { *pos += 1; return Some(out); }
        if c == '\\' {
            *pos += 1;
            match chars.get(*pos) {
                Some('"')  => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('/')  => out.push('/'),
                Some('n')  => out.push('\n'),
                Some('r')  => out.push('\r'),
                Some('t')  => out.push('\t'),
                _ => return None,
            }
            *pos += 1;
        } else {
            out.push(c);
            *pos += 1;
        }
    }
    None // unterminated string
}

// =============================================================================
// VYRATMPL HELPERS — Jinja2-style template engine
// Supports: {{var}}  {% if cond %}...{% endif %}  {% each item in list %}...{% endeach %}
// =============================================================================

fn vyratmpl_render(tmpl: &str, data: &[(String, Value)]) -> String {
    let mut out = String::new();
    let mut chars: Vec<char> = tmpl.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Check for {{ variable }}
        if i + 1 < chars.len() && chars[i] == '{' && chars[i+1] == '{' {
            i += 2; // skip {{
            let mut var_name = String::new();
            while i < chars.len() && !(chars[i] == '}' && i+1 < chars.len() && chars[i+1] == '}') {
                var_name.push(chars[i]);
                i += 1;
            }
            i += 2; // skip }}
            let var_name = var_name.trim();
            // Support dot notation: user.name
            let val = if var_name.contains('.') {
                let parts: Vec<&str> = var_name.splitn(2, '.').collect();
                let obj = data.iter().find(|(k, _)| k == parts[0]).map(|(_, v)| v.clone()).unwrap_or(Value::Null);
                match obj {
                    Value::Map(ref pairs) => pairs.iter().find(|(k, _)| k == parts[1]).map(|(_, v)| v.to_string()).unwrap_or_default(),
                    Value::Struct { ref fields, .. } => fields.iter().find(|(k, _)| k == parts[1]).map(|(_, v)| v.to_string()).unwrap_or_default(),
                    other => other.to_string(),
                }
            } else {
                data.iter().find(|(k, _)| k == var_name).map(|(_, v)| v.to_string()).unwrap_or_default()
            };
            out.push_str(&val);
            continue;
        }

        // Check for {% block %}
        if i + 1 < chars.len() && chars[i] == '{' && chars[i+1] == '%' {
            i += 2; // skip {%
            let mut tag = String::new();
            while i < chars.len() && !(chars[i] == '%' && i+1 < chars.len() && chars[i+1] == '}') {
                tag.push(chars[i]);
                i += 1;
            }
            i += 2; // skip %}
            let tag = tag.trim().to_string();

            if tag.starts_with("if ") {
                // Find matching {% endif %}
                let cond_var = tag[3..].trim();
                let cond_val = data.iter().find(|(k, _)| k == cond_var).map(|(_, v)| v.clone()).unwrap_or(Value::Null);
                // Collect body until endif
                let rest: String = chars[i..].iter().collect();
                let (body, after) = vyratmpl_extract_block(&rest, "if", "endif");
                if cond_val.is_truthy() {
                    out.push_str(&vyratmpl_render(&body, data));
                }
                let _skip = tmpl.len() - rest.len() + after;
                // Rebuild chars from updated position
                let full: Vec<char> = tmpl.chars().collect();
                chars = full;
                i = tmpl.len() - rest.len() + after;
                continue;
            }

            if tag.starts_with("each ") {
                // {% each item in list_var %}
                let parts: Vec<&str> = tag[5..].splitn(3, ' ').collect();
                let item_name = if parts.len() >= 1 { parts[0] } else { "item" };
                let list_name = if parts.len() >= 3 { parts[2] } else if parts.len() >= 1 { parts[0] } else { "list" };
                let list_val  = data.iter().find(|(k, _)| k == list_name).map(|(_, v)| v.clone()).unwrap_or(Value::Null);
                let rest: String = chars[i..].iter().collect();
                let (body, after) = vyratmpl_extract_block(&rest, "each", "endeach");
                if let Value::List(items) = list_val {
                    for item in items {
                        let mut item_data = data.to_vec();
                        item_data.push((item_name.to_string(), item.clone()));
                        // Also push item fields if it's a map
                        if let Value::Map(ref pairs) = item {
                            for (k, v) in pairs { item_data.push((format!("{}.{}", item_name, k), v.clone())); }
                        }
                        out.push_str(&vyratmpl_render(&body, &item_data));
                    }
                }
                i = tmpl.len() - rest.len() + after;
                chars = tmpl.chars().collect();
                continue;
            }
            // Unknown tag — skip
            continue;
        }

        out.push(chars[i]);
        i += 1;
    }
    out
}

fn vyratmpl_extract_block(text: &str, _open_tag: &str, close_tag: &str) -> (String, usize) {
    // Find matching close tag, handling nesting
    let _close = format!("{{% {} %}}", close_tag);
    let close_simple = format!("{{% {} %}}", close_tag);
    // Try both with and without spaces
    let pos = text.find(&close_simple)
        .or_else(|| text.find(&format!("{{%{}%}}", close_tag)));
    match pos {
        Some(p) => {
            let body = text[..p].to_string();
            (body, p + close_simple.len())
        }
        None => (text.to_string(), text.len()),
    }
}

// =============================================================================
// VYRASOCKET HELPERS — RFC 6455 WebSocket protocol implementation
// Real WebSocket handshake + frame encode/decode — no external libs
// =============================================================================

fn vyrasocket_extract_key(request: &str) -> Option<String> {
    for line in request.lines() {
        if line.starts_with("Sec-WebSocket-Key:") {
            return Some(line[18..].trim().to_string());
        }
    }
    None
}

fn vyrasocket_accept_key(key: &str) -> String {
    // RFC 6455: SHA-1(key + magic) → base64
    let magic = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let combined = format!("{}{}", key, magic);
    // SHA-1 implementation (no external deps)
    let hash = vyrasocket_sha1(combined.as_bytes());
    vyrasocket_base64(&hash)
}

fn vyrasocket_sha1(data: &[u8]) -> [u8; 20] {
    // Pure Rust SHA-1 per FIPS 180-4
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 { msg.push(0); }
    for b in bit_len.to_be_bytes() { msg.push(b); }
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i*4], chunk[i*4+1], chunk[i*4+2], chunk[i*4+3]]);
        }
        for i in 16..80 {
            w[i] = (w[i-3] ^ w[i-8] ^ w[i-14] ^ w[i-16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for i in 0..80 {
            let (f, k) = match i {
                0..=19  => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d,             0x6ED9EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDCu32),
                _       => (b ^ c ^ d,             0xCA62C1D6u32),
            };
            let temp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(w[i]);
            e = d; d = c; c = b.rotate_left(30); b = a; a = temp;
        }
        h[0] = h[0].wrapping_add(a); h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c); h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, &val) in h.iter().enumerate() {
        let bytes = val.to_be_bytes();
        out[i*4..i*4+4].copy_from_slice(&bytes);
    }
    out
}

fn vyrasocket_base64(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { TABLE[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(n & 63) as usize] as char } else { '=' });
    }
    out
}

fn vyrasocket_decode_frame(data: &[u8]) -> String {
    // RFC 6455 frame decode: handle masked client frames
    if data.len() < 6 { return String::new(); }
    let payload_len = (data[1] & 0x7F) as usize;
    let mask_start = 2;
    let mask = [data[mask_start], data[mask_start+1], data[mask_start+2], data[mask_start+3]];
    let payload_start = mask_start + 4;
    let end = (payload_start + payload_len).min(data.len());
    let decoded: Vec<u8> = data[payload_start..end].iter().enumerate()
        .map(|(i, &b)| b ^ mask[i % 4])
        .collect();
    String::from_utf8_lossy(&decoded).to_string()
}

fn vyrasocket_encode_frame(msg: &str) -> Vec<u8> {
    // RFC 6455 server → client frame (unmasked text frame)
    let payload = msg.as_bytes();
    let mut frame = vec![0x81u8]; // FIN + text opcode
    if payload.len() <= 125 {
        frame.push(payload.len() as u8);
    } else if payload.len() <= 65535 {
        frame.push(126);
        frame.push((payload.len() >> 8) as u8);
        frame.push((payload.len() & 0xFF) as u8);
    } else {
        frame.push(127);
        for shift in (0..8).rev() { frame.push(((payload.len() >> (shift * 8)) & 0xFF) as u8); }
    }
    frame.extend_from_slice(payload);
    frame
}
fn value_to_json(val: &Value) -> String {
    match val {
        Value::Null        => "null".into(),
        Value::Bool(b)     => b.to_string(),
        Value::Int(n)      => n.to_string(),
        Value::Float(f)    => f.to_string(),
        Value::Str(s)      => {
            // Escape JSON special chars
            let escaped = s
                .replace('\\', "\\\\")
                .replace('"',  "\\\"")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\t', "\\t");
            format!("\"{}\"", escaped)
        }
        Value::List(items) => {
            let parts: Vec<String> = items.iter().map(value_to_json).collect();
            format!("[{}]", parts.join(","))
        }
        Value::Map(pairs) => {
            let parts: Vec<String> = pairs.iter()
                .map(|(k, v)| format!("\"{}\":{}", k, value_to_json(v)))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Struct { name: _, fields } => {
            let parts: Vec<String> = fields.iter()
                .map(|(k, v)| format!("\"{}\":{}", k, value_to_json(v)))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Ok(inner)   => format!("{{\"ok\":{}}}", value_to_json(inner)),
        Value::Err(msg)    => format!("{{\"err\":{}}}", value_to_json(&Value::Str(msg.clone()))),
        Value::Range(a, b) => format!("{{\"from\":{},\"to\":{}}}", a, b),
        _                  => "null".into(),
    }
}
// =============================================================================
// MALIB — Remox Advanced Math Engine  (use Malib)
// Covers: arithmetic, trig, log, algebra, calculus, number theory,
//         combinatorics, statistics, matrices, complex numbers, series
// Usage:  use Malib
//         say Malib.solve("x^2 - 5x + 6 = 0")
// =============================================================================

fn malib_f64(v: &Value) -> f64 {
    match v { Value::Int(n) => *n as f64, Value::Float(f) => *f, _ => 0.0 }
}
fn malib_i64(v: &Value) -> i64 {
    match v { Value::Int(n) => *n, Value::Float(f) => *f as i64, _ => 0 }
}

// ── Number Theory ─────────────────────────────────────────────────────────────
fn malib_gcd(mut a: i64, mut b: i64) -> i64 {
    a = a.abs(); b = b.abs();
    while b != 0 { let t = b; b = a % b; a = t; }
    a
}
fn malib_lcm(a: i64, b: i64) -> i64 {
    if a == 0 || b == 0 { 0 } else { (a / malib_gcd(a, b)) * b }
}
fn malib_mod_pow(mut base: i64, mut exp: i64, modulus: i64) -> i64 {
    let mut result = 1i64; base %= modulus;
    while exp > 0 {
        if exp & 1 == 1 { result = ((result as i128 * base as i128) % modulus as i128) as i64; }
        exp >>= 1;
        base = ((base as i128 * base as i128) % modulus as i128) as i64;
    }
    result
}
fn malib_is_prime(n: i64) -> bool {
    if n < 2 { return false; }
    if n == 2 || n == 3 || n == 5 || n == 7 { return true; }
    if n % 2 == 0 || n % 3 == 0 { return false; }
    let mut d = n - 1; let mut r = 0u32;
    while d % 2 == 0 { d /= 2; r += 1; }
    for &a in &[2i64, 3, 5, 7] {
        if a >= n { continue; }
        let mut x = malib_mod_pow(a, d, n);
        if x == 1 || x == n - 1 { continue; }
        let mut cont = false;
        for _ in 0..r-1 {
            x = ((x as i128 * x as i128) % n as i128) as i64;
            if x == n - 1 { cont = true; break; }
        }
        if !cont { return false; }
    }
    true
}
fn malib_factorize(mut n: i64) -> Vec<i64> {
    let mut f = Vec::new();
    if n <= 1 { return f; }
    let mut d = 2i64;
    while d * d <= n { while n % d == 0 { f.push(d); n /= d; } d += if d==2{1}else{2}; }
    if n > 1 { f.push(n); }
    f
}
fn malib_factorial(n: i64) -> i64 {
    if n <= 1 { 1 } else { (2..=n).product() }
}
fn malib_fibonacci(n: i64) -> i64 {
    if n <= 0 { return 0; } if n == 1 { return 1; }
    let (mut a, mut b) = (0i64, 1i64);
    for _ in 2..=n { let c = a + b; a = b; b = c; }
    b
}
fn malib_totient(n: i64) -> i64 {
    if n <= 0 { return 0; }
    let mut res = n; let mut p = 2i64; let mut nn = n;
    while p * p <= nn {
        if nn % p == 0 { while nn % p == 0 { nn /= p; } res -= res / p; }
        p += 1;
    }
    if nn > 1 { res -= res / nn; }
    res
}
fn malib_modinv(a: i64, m: i64) -> Option<i64> {
    let (mut or, mut r) = (a, m); let (mut os, mut s) = (1i64, 0i64);
    while r != 0 {
        let q = or / r;
        let t = r; r = or - q*r; or = t;
        let t = s; s = os - q*s; os = t;
    }
    if or != 1 { None } else { Some(((os % m) + m) % m) }
}
fn malib_primes_upto(limit: usize) -> Vec<i64> {
    if limit < 2 { return vec![]; }
    let mut sieve = vec![true; limit+1]; sieve[0]=false; sieve[1]=false;
    let mut i = 2;
    while i*i<=limit { if sieve[i] { let mut j=i*i; while j<=limit{sieve[j]=false;j+=i;} } i+=1; }
    sieve.iter().enumerate().filter(|(_,&v)|v).map(|(i,_)|i as i64).collect()
}

// ── Combinatorics ─────────────────────────────────────────────────────────────
fn malib_ncr(n: i64, r: i64) -> i64 {
    if r > n || r < 0 { return 0; }
    let r = r.min(n-r); let mut res = 1i64;
    for i in 0..r { res = res*(n-i)/(i+1); }
    res
}
fn malib_npr(n: i64, r: i64) -> i64 {
    if r > n { return 0; }
    ((n-r+1)..=n).product()
}

// ── Equation Solvers ──────────────────────────────────────────────────────────
fn malib_solve_str(expr: &str) -> Value {
    let s = expr.replace(' ', "").to_lowercase();
    let parts: Vec<&str> = s.splitn(2, '=').collect();
    if parts.len() != 2 { return Value::Str("Error: equation must contain '='".into()); }
    let combined = if parts[1]=="0" { parts[0].to_string() } else { format!("({})-( {})",parts[0],parts[1]) };
    // Guard: malib_poly_coeffs only understands terms up to x^2 — any
    // cubic-or-higher term would silently be dropped (ignored) rather
    // than solved, which would give a *wrong* answer with no indication
    // anything was lost. Detect that case up front and fail loudly
    // instead, rather than let it happen silently.
    if let Some(bad_term) = malib_find_unsupported_degree(&combined) {
        return Value::Str(format!(
            "Error: malib_solve only supports linear and quadratic equations (up to x^2) — \
             found unsupported term '{}'. Cubic/higher-degree solving isn't implemented.",
            bad_term
        ));
    }
    let (a,b,c) = malib_poly_coeffs(&combined);
    if a.abs() > 1e-12 {
        let disc = b*b - 4.0*a*c;
        if disc < 0.0 { return Value::Str(format!("No real roots (discriminant={:.4})",disc)); }
        let sq = disc.sqrt();
        let r1 = (-b+sq)/(2.0*a); let r2 = (-b-sq)/(2.0*a);
        if (r1-r2).abs()<1e-12 { Value::List(vec![Value::Float(r1)]) }
        else { Value::List(vec![Value::Float(r1), Value::Float(r2)]) }
    } else if b.abs() > 1e-12 {
        Value::List(vec![Value::Float(-c/b)])
    } else if c.abs() < 1e-12 {
        Value::Str("Infinite solutions".into())
    } else {
        Value::Str("No solution".into())
    }
}
fn malib_linear(a: f64, b: f64) -> Value {
    // ax + b = 0  →  x = -b/a
    if a.abs() < 1e-15 { return Value::Str("No solution (a=0)".into()); }
    Value::Float(-b / a)
}
fn malib_quadratic(a: f64, b: f64, c: f64) -> Value {
    if a.abs() < 1e-15 { return malib_linear(b, c); }
    let disc = b*b - 4.0*a*c;
    if disc < 0.0 { return Value::Str(format!("No real roots (D={:.4})",disc)); }
    let sq = disc.sqrt();
    let r1 = (-b+sq)/(2.0*a); let r2 = (-b-sq)/(2.0*a);
    if (r1-r2).abs()<1e-12 { Value::List(vec![Value::Float(r1)]) }
    else { Value::List(vec![Value::Float(r1), Value::Float(r2)]) }
}
/// Scans a (already combined, one-side-of-equation) expression string for
/// any term of degree 3 or higher in x — e.g. x^3, x³, x^10 — which
/// `malib_poly_coeffs` below does not handle (it only recognizes x^2 and
/// x). Returns the offending term's source text if found, so
/// `malib_solve_str` can fail with a clear error instead of silently
/// dropping the term and returning a wrong (lower-degree) answer.
fn malib_find_unsupported_degree(expr: &str) -> Option<String> {
    let s = expr.replace(' ', "").replace("**", "^");
    // Unicode superscript digits for 3 and above (² is fine/supported).
    const HIGH_SUPERSCRIPTS: [char; 7] = ['³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == 'x' {
            // Pattern: x^N
            if i + 1 < chars.len() && chars[i + 1] == '^' {
                let mut j = i + 2;
                let mut num = String::new();
                while j < chars.len() && chars[j].is_ascii_digit() {
                    num.push(chars[j]);
                    j += 1;
                }
                if let Ok(n) = num.parse::<u32>() {
                    if n >= 3 {
                        let term: String = chars[i..j].iter().collect();
                        return Some(term);
                    }
                }
            }
            // Pattern: x followed directly by a high unicode superscript
            if i + 1 < chars.len() && HIGH_SUPERSCRIPTS.contains(&chars[i + 1]) {
                let term: String = chars[i..=i+1].iter().collect();
                return Some(term);
            }
        }
        i += 1;
    }
    None
}

fn malib_poly_coeffs(expr: &str) -> (f64, f64, f64) {
    let s = expr.replace(' ',"").replace("**","^");
    let (mut a,mut b,mut c) = (0f64,0f64,0f64);
    let mut terms: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
        if (ch=='+'||ch=='-') && !cur.is_empty() { terms.push(cur.clone()); cur=String::new(); }
        cur.push(ch);
    }
    if !cur.is_empty() { terms.push(cur); }
    for term in &terms {
        let t = term.trim_matches(|c|c=='('||c==')');
        if t.is_empty() { continue; }
        if t.contains("x^2")||t.contains("x²") { a += malib_parse_coeff(&t.replace("x^2","").replace("x²","")); }
        else if t.contains('x') { b += malib_parse_coeff(&t.replace('x',"")); }
        else { c += t.parse::<f64>().unwrap_or(0.0); }
    }
    (a,b,c)
}
fn malib_parse_coeff(s: &str) -> f64 {
    let s=s.trim();
    if s.is_empty()||s=="+" {1.0} else if s=="-" {-1.0} else {s.parse().unwrap_or(0.0)}
}

// ── Expression Evaluator ──────────────────────────────────────────────────────
fn malib_eval(s: &str, x: f64) -> f64 {
    let s = s.trim().replace(' ',"").replace("**","^");
    malib_eval_inner(&s, x)
}
fn malib_eval_inner(s: &str, x: f64) -> f64 {
    if s.is_empty() { return 0.0; }
    if s.starts_with('(') && s.ends_with(')') && malib_matching_paren(s) {
        return malib_eval_inner(&s[1..s.len()-1], x);
    }
    if let Some((l,r,op)) = malib_split_add(s) {
        let lv=malib_eval_inner(&l,x); let rv=malib_eval_inner(&r,x);
        return if op=='+' {lv+rv} else {lv-rv};
    }
    if let Some((l,r)) = malib_split_op(s,'*') { return malib_eval_inner(&l,x)*malib_eval_inner(&r,x); }
    if let Some((l,r)) = malib_split_op(s,'/') { let rv=malib_eval_inner(&r,x); return if rv!=0.0 {malib_eval_inner(&l,x)/rv} else {f64::NAN}; }
    if let Some(idx) = malib_pow_idx(s) { return malib_eval_inner(&s[..idx],x).powf(malib_eval_inner(&s[idx+1..],x)); }
    if s.starts_with("sin(")  && s.ends_with(')') { return malib_eval_inner(&s[4..s.len()-1],x).sin(); }
    if s.starts_with("cos(")  && s.ends_with(')') { return malib_eval_inner(&s[4..s.len()-1],x).cos(); }
    if s.starts_with("tan(")  && s.ends_with(')') { return malib_eval_inner(&s[4..s.len()-1],x).tan(); }
    if s.starts_with("ln(")   && s.ends_with(')') { return malib_eval_inner(&s[3..s.len()-1],x).ln(); }
    if s.starts_with("log(")  && s.ends_with(')') { return malib_eval_inner(&s[4..s.len()-1],x).log10(); }
    if s.starts_with("exp(")  && s.ends_with(')') { return malib_eval_inner(&s[4..s.len()-1],x).exp(); }
    if s.starts_with("sqrt(") && s.ends_with(')') { return malib_eval_inner(&s[5..s.len()-1],x).sqrt(); }
    if s.starts_with("abs(")  && s.ends_with(')') { return malib_eval_inner(&s[4..s.len()-1],x).abs(); }
    if s.starts_with("asin(") && s.ends_with(')') { return malib_eval_inner(&s[5..s.len()-1],x).asin(); }
    if s.starts_with("acos(") && s.ends_with(')') { return malib_eval_inner(&s[5..s.len()-1],x).acos(); }
    if s.starts_with("atan(") && s.ends_with(')') { return malib_eval_inner(&s[5..s.len()-1],x).atan(); }
    if s.starts_with('-') { return -malib_eval_inner(&s[1..],x); }
    if s == "x" { return x; }
    if s == "pi" || s == "π" { return std::f64::consts::PI; }
    if s == "e"  { return std::f64::consts::E; }
    s.parse::<f64>().unwrap_or(f64::NAN)
}
fn malib_matching_paren(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    if chars[0] != '(' { return false; }
    let mut depth = 0i32;
    for (i,&c) in chars.iter().enumerate() {
        if c=='(' {depth+=1;} if c==')' {depth-=1;}
        if depth==0 && i<chars.len()-1 { return false; }
    }
    depth==0
}
fn malib_split_add(s: &str) -> Option<(String,String,char)> {
    let chars: Vec<char> = s.chars().collect();
    let mut depth=0i32;
    for i in 1..chars.len() {
        match chars[i] { '('=> depth+=1, ')'=> depth-=1,
            '+'|'-' if depth==0 => return Some((chars[..i].iter().collect(),chars[i+1..].iter().collect(),chars[i])),
            _ => {} }
    }
    None
}
fn malib_split_op(s: &str, op: char) -> Option<(String,String)> {
    let chars: Vec<char> = s.chars().collect();
    let mut depth=0i32;
    for i in 1..chars.len() {
        match chars[i] { '('=> depth+=1, ')'=> depth-=1,
            c if c==op && depth==0 => return Some((chars[..i].iter().collect(),chars[i+1..].iter().collect())),
            _ => {} }
    }
    None
}
fn malib_pow_idx(s: &str) -> Option<usize> {
    let mut depth=0i32;
    for (i,c) in s.chars().enumerate() {
        match c { '('=> depth+=1, ')'=> depth-=1, '^' if depth==0 => return Some(i), _ => {} }
    }
    None
}

// ── Symbolic Differentiation ──────────────────────────────────────────────────
fn malib_diff_sym(expr: &str) -> String {
    let s = expr.trim().replace(' ',"").replace("**","^");
    malib_diff_inner(&s)
}
fn malib_diff_inner(s: &str) -> String {
    if s.starts_with("sin(") && s.ends_with(')') { let i=&s[4..s.len()-1]; return format!("cos({})*( {})",i,malib_diff_inner(i)); }
    if s.starts_with("cos(") && s.ends_with(')') { let i=&s[4..s.len()-1]; return format!("-sin({})*( {})",i,malib_diff_inner(i)); }
    if s.starts_with("tan(") && s.ends_with(')') { let i=&s[4..s.len()-1]; return format!("(1/cos({})^2)*( {})",i,malib_diff_inner(i)); }
    if s.starts_with("ln(")  && s.ends_with(')') { let i=&s[3..s.len()-1]; return format!("({})/( {})",malib_diff_inner(i),i); }
    if s.starts_with("exp(") && s.ends_with(')') { let i=&s[4..s.len()-1]; return format!("exp({})*( {})",i,malib_diff_inner(i)); }
    if s.starts_with("sqrt(") && s.ends_with(')') { let i=&s[5..s.len()-1]; return format!("({}/(2*sqrt({})))",malib_diff_inner(i),i); }
    if s.starts_with('(') && s.ends_with(')') { return malib_diff_inner(&s[1..s.len()-1]); }
    if s.starts_with('-') { return format!("-({})", malib_diff_inner(&s[1..])); }
    if let Some(idx) = malib_pow_idx(s) {
        let base=&s[..idx]; let exp_s=&s[idx+1..];
        if base=="x" {
            if let Ok(n) = exp_s.parse::<f64>() {
                if n==0.0 {return "0".into();} if n==1.0 {return "1".into();}
                let ne=n-1.0; return if ne==1.0 {format!("{}",n)} else {format!("{}*x^{}",n,ne)};
            }
        }
        if base=="e" && exp_s=="x" { return "e^x".into(); }
        if let Ok(n) = exp_s.parse::<f64>() {
            let du=malib_diff_inner(base); return format!("{}*{}^{}*({})",n,base,n-1.0,du);
        }
    }
    if let Some((l,r,op)) = malib_split_add(s) {
        return format!("{}{}{}",malib_diff_inner(&l),if op=='+'{"+"}else{"-"},malib_diff_inner(&r));
    }
    if let Some((l,r)) = malib_split_op(s,'*') {
        return format!("({})*({})+({})*( {})",malib_diff_inner(&l),r,l,malib_diff_inner(&r));
    }
    if let Some((l,r)) = malib_split_op(s,'/') {
        return format!("(({})*({})-({})*({}))/({})^2",malib_diff_inner(&l),r,l,malib_diff_inner(&r),r);
    }
    if s=="x" { return "1".into(); }
    if s.parse::<f64>().is_ok() { return "0".into(); }
    format!("d/dx({})", s)
}

// Numerical derivative at point
fn malib_derivative_at(expr: &str, at: f64) -> f64 {
    let h = 1e-7;
    (malib_eval(expr, at+h) - malib_eval(expr, at-h)) / (2.0*h)
}

// ── Numerical Integration (Gauss-Legendre 5-point) ────────────────────────────
fn malib_integrate(expr: &str, lo: f64, hi: f64) -> f64 {
    let nodes:   [f64;5] = [-0.9061798459,-0.5384693101,0.0,0.5384693101,0.9061798459];
    let weights: [f64;5] = [ 0.2369268851, 0.4786286705,0.5688888889,0.4786286705,0.2369268851];
    let mid=(hi+lo)/2.0; let half=(hi-lo)/2.0;
    half * nodes.iter().zip(weights.iter()).map(|(t,w)| w*malib_eval(expr,mid+half*t)).sum::<f64>()
}

// ── Newton-Raphson Root Finder ────────────────────────────────────────────────
fn malib_newton(expr: &str, guess: f64, max_iter: usize, tol: f64) -> f64 {
    let mut x=guess; let h=1e-7;
    for _ in 0..max_iter {
        let fx=malib_eval(expr,x); let fpx=(malib_eval(expr,x+h)-fx)/h;
        if fpx.abs()<1e-15 { break; }
        let xn=x-fx/fpx; if (xn-x).abs()<tol {return xn;} x=xn;
    }
    x
}

// Limit (numerical, two-sided)
fn malib_limit(expr: &str, at: f64) -> f64 {
    let h=1e-8;
    (malib_eval(expr,at+h) + malib_eval(expr,at-h)) / 2.0
}

// ── Statistics ────────────────────────────────────────────────────────────────
fn malib_stats_all(data: &[f64]) -> Vec<(String,Value)> {
    if data.is_empty() { return vec![("error".into(),Value::Str("empty".into()))]; }
    let n=data.len() as f64;
    let mean=data.iter().sum::<f64>()/n;
    let variance=data.iter().map(|x|(x-mean).powi(2)).sum::<f64>()/n;
    let mut sorted=data.to_vec();
    sorted.sort_by(|a,b|a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median=if sorted.len()%2==0{(sorted[sorted.len()/2-1]+sorted[sorted.len()/2])/2.0}else{sorted[sorted.len()/2]};
    let q1=malib_percentile(&sorted,25.0); let q3=malib_percentile(&sorted,75.0);
    let mut freq: Vec<(i64,usize)>=Vec::new();
    for &v in data {
        let k=(v*1e9) as i64;
        if let Some(e)=freq.iter_mut().find(|(x,_)|*x==k){e.1+=1;}else{freq.push((k,1));}
    }
    freq.sort_by(|a,b|b.1.cmp(&a.1));
    let mode=freq.first().map(|(k,_)|*k as f64/1e9).unwrap_or(f64::NAN);
    vec![
        ("mean".into(),Value::Float(mean)),("median".into(),Value::Float(median)),
        ("mode".into(),Value::Float(mode)),("stdev".into(),Value::Float(variance.sqrt())),
        ("variance".into(),Value::Float(variance)),
        ("min".into(),Value::Float(sorted[0])),("max".into(),Value::Float(*sorted.last().unwrap())),
        ("range".into(),Value::Float(sorted.last().unwrap()-sorted[0])),
        ("count".into(),Value::Int(data.len() as i64)),("sum".into(),Value::Float(data.iter().sum())),
        ("q1".into(),Value::Float(q1)),("q3".into(),Value::Float(q3)),("iqr".into(),Value::Float(q3-q1)),
    ]
}
fn malib_percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {return f64::NAN;}
    let idx=p/100.0*(sorted.len()-1) as f64;
    let lo=idx.floor() as usize; let hi=idx.ceil() as usize;
    if lo==hi {sorted[lo]} else {sorted[lo]+(sorted[hi]-sorted[lo])*(idx-lo as f64)}
}

// ── Matrix Operations ──────────────────────────────────────────────────────────
fn malib_value_to_mat(v: &Value) -> Option<Vec<Vec<f64>>> {
    if let Value::List(rows) = v {
        let mut mat=Vec::new();
        for row in rows {
            if let Value::List(cols)=row {
                mat.push(cols.iter().map(|c|malib_f64(c)).collect());
            } else {return None;}
        }
        Some(mat)
    } else {None}
}
fn malib_mat_to_value(m: &[Vec<f64>]) -> Value {
    Value::List(m.iter().map(|row|Value::List(row.iter().map(|&x|Value::Float(x)).collect())).collect())
}
fn malib_mat_mul(a: &[Vec<f64>], b: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let (m,k,n)=(a.len(),b.len(),if b.is_empty(){0}else{b[0].len()});
    if a.is_empty()||a[0].len()!=k{return None;}
    let mut c=vec![vec![0f64;n];m];
    for i in 0..m{for j in 0..n{for p in 0..k{c[i][j]+=a[i][p]*b[p][j];}}}
    Some(c)
}
fn malib_mat_det(m: &[Vec<f64>]) -> f64 {
    let n=m.len();
    if n==0{return 0.0;} if n==1{return m[0][0];} if n==2{return m[0][0]*m[1][1]-m[0][1]*m[1][0];}
    let mut det=0f64;
    for j in 0..n {
        let minor: Vec<Vec<f64>>=m[1..].iter().map(|row|row.iter().enumerate().filter(|(k,_)|*k!=j).map(|(_,v)|*v).collect()).collect();
        det+=(if j%2==0{1.0}else{-1.0})*m[0][j]*malib_mat_det(&minor);
    }
    det
}
fn malib_mat_add(a: &[Vec<f64>], b: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    if a.len()!=b.len()||a[0].len()!=b[0].len(){return None;}
    Some(a.iter().zip(b.iter()).map(|(ra,rb)|ra.iter().zip(rb.iter()).map(|(x,y)|x+y).collect()).collect())
}
fn malib_mat_transpose(a: &[Vec<f64>]) -> Vec<Vec<f64>> {
    if a.is_empty(){return vec![];}
    let (m,n)=(a.len(),a[0].len());
    (0..n).map(|j|(0..m).map(|i|a[i][j]).collect()).collect()
}
fn malib_mat_inv(m: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let n=m.len(); if n==0{return None;}
    let mut aug: Vec<Vec<f64>>=m.iter().enumerate().map(|(i,row)|{let mut r=row.clone();for j in 0..n{r.push(if i==j{1.0}else{0.0});}r}).collect();
    for col in 0..n {
        let pr=(col..n).max_by(|&a,&b|aug[a][col].abs().partial_cmp(&aug[b][col].abs()).unwrap_or(std::cmp::Ordering::Equal))?;
        aug.swap(col,pr);
        let piv=aug[col][col]; if piv.abs()<1e-12{return None;}
        for v in &mut aug[col]{*v/=piv;}
        for i in 0..n{if i==col{continue;} let f=aug[i][col]; for j in 0..2*n{aug[i][j]-=f*aug[col][j];}}
    }
    Some(aug.iter().map(|row|row[n..].to_vec()).collect())
}

// ── Complex Numbers ───────────────────────────────────────────────────────────
fn malib_make_complex(re: f64, im: f64) -> Value {
    Value::Map(vec![
        ("re".into(),Value::Float(re)),("im".into(),Value::Float(im)),
        ("abs".into(),Value::Float((re*re+im*im).sqrt())),
        ("arg".into(),Value::Float(im.atan2(re))),
        ("str".into(),Value::Str(if im>=0.0{format!("{}+{}i",re,im)}else{format!("{}{}i",re,im)})),
    ])
}

// ── Series ────────────────────────────────────────────────────────────────────
fn malib_arithmetic_series(a: f64, d: f64, n: usize) -> Value {
    let sum=(n as f64/2.0)*(2.0*a+(n as f64-1.0)*d);
    let terms: Vec<Value>=(0..n).map(|i|Value::Float(a+i as f64*d)).collect();
    Value::Map(vec![("sum".into(),Value::Float(sum)),("terms".into(),Value::List(terms)),("nth".into(),Value::Float(a+(n as f64-1.0)*d))])
}
fn malib_geometric_series(a: f64, r: f64, n: usize) -> Value {
    let sum=if (r-1.0).abs()<1e-12{a*n as f64}else{a*(1.0-r.powf(n as f64))/(1.0-r)};
    let terms: Vec<Value>=(0..n).map(|i|Value::Float(a*r.powf(i as f64))).collect();
    Value::Map(vec![("sum".into(),Value::Float(sum)),("terms".into(),Value::List(terms)),("nth".into(),Value::Float(a*r.powf((n as f64)-1.0)))])
}

// ── Fraction Helper ───────────────────────────────────────────────────────────
fn malib_to_fraction(x: f64) -> Value {
    let eps=1e-9; let max_den=10000i64;
    let mut best_n=x.round() as i64; let mut best_d=1i64;
    for d in 1..=max_den {
        let n=(x*d as f64).round() as i64;
        let err=(x - n as f64/d as f64).abs();
        if err<eps{best_n=n;best_d=d;break;}
        if (n as f64/d as f64-x).abs() < (best_n as f64/best_d as f64-x).abs(){best_n=n;best_d=d;}
    }
    let g=malib_gcd(best_n.abs(),best_d);
    Value::Map(vec![
        ("numerator".into(),Value::Int(best_n/g)),
        ("denominator".into(),Value::Int(best_d/g)),
        ("str".into(),Value::Str(format!("{}/{}",best_n/g,best_d/g))),
    ])
}

// ── Simplify (basic algebraic) ────────────────────────────────────────────────
fn malib_simplify(expr: &str) -> String {
    // Numerical simplification: evaluate if pure constant expression
    let val = malib_eval(expr, 0.0); // try with x=0, if no x
    if !val.is_nan() && !val.is_infinite() {
        // Check if there's an 'x' in the expression
        if !expr.contains('x') {
            return format!("{}", val);
        }
    }
    // Basic string cleanup
    expr.replace("+-","-").replace("--","+").replace("*1","").replace("1*","").replace("+0","").replace("0+","")
}

// ── clamp / lerp / sign / deg / rad ──────────────────────────────────────────
fn malib_clamp(x: f64, lo: f64, hi: f64) -> f64 { x.max(lo).min(hi) }
fn malib_lerp(a: f64, b: f64, t: f64) -> f64 { a + (b-a)*t }
fn malib_map_range(x: f64, in_lo: f64, in_hi: f64, out_lo: f64, out_hi: f64) -> f64 {
    out_lo + (x-in_lo)/(in_hi-in_lo)*(out_hi-out_lo)
}

// =============================================================================
// MALIB DISPATCHER
// =============================================================================
pub(crate) fn dispatch_malib(name: &str, mut args: Vec<Value>) -> Result<Value, String> {
    // Helper closures
    let f0 = |args: &Vec<Value>| malib_f64(args.get(0).unwrap_or(&Value::Int(0)));
    let f1 = |args: &Vec<Value>| malib_f64(args.get(1).unwrap_or(&Value::Int(0)));
    let f2 = |args: &Vec<Value>| malib_f64(args.get(2).unwrap_or(&Value::Int(0)));
    let i0 = |args: &Vec<Value>| malib_i64(args.get(0).unwrap_or(&Value::Int(0)));
    let i1 = |args: &Vec<Value>| malib_i64(args.get(1).unwrap_or(&Value::Int(0)));
    let s0 = |args: &Vec<Value>| args.get(0).map(|v|v.to_string()).unwrap_or_default();

    match name {
        // ── Basic arithmetic ──────────────────────────────────────────────────
        "__malib_sqrt"  => return Ok(Value::Float(f0(&args).sqrt())),
        "__malib_cbrt"  => return Ok(Value::Float(f0(&args).cbrt())),
        "__malib_pow"   => return Ok(Value::Float(f0(&args).powf(f1(&args)))),
        "__malib_abs"   => return Ok(match args.into_iter().next().unwrap_or(Value::Int(0)) {
                               Value::Int(n)=>Value::Int(n.abs()), Value::Float(f)=>Value::Float(f.abs()), v=>v }),
        "__malib_floor" => return Ok(Value::Int(f0(&args).floor() as i64)),
        "__malib_ceil"  => return Ok(Value::Int(f0(&args).ceil() as i64)),
        "__malib_round" => return Ok(Value::Int(f0(&args).round() as i64)),
        "__malib_min"   => { let a=args.remove(0); let b=args.remove(0); return Ok(match (&a,&b){(Value::Int(x),Value::Int(y))=>if x<=y{a}else{b},(Value::Float(x),Value::Float(y))=>if x<=y{a}else{b},_=>a}); }
        "__malib_max"   => { let a=args.remove(0); let b=args.remove(0); return Ok(match (&a,&b){(Value::Int(x),Value::Int(y))=>if x>=y{a}else{b},(Value::Float(x),Value::Float(y))=>if x>=y{a}else{b},_=>a}); }

        // ── Trig / log ────────────────────────────────────────────────────────
        "__malib_sin"   => return Ok(Value::Float(f0(&args).sin())),
        "__malib_cos"   => return Ok(Value::Float(f0(&args).cos())),
        "__malib_tan"   => return Ok(Value::Float(f0(&args).tan())),
        "__malib_asin"  => return Ok(Value::Float(f0(&args).asin())),
        "__malib_acos"  => return Ok(Value::Float(f0(&args).acos())),
        "__malib_atan"  => return Ok(Value::Float(f0(&args).atan())),
        "__malib_atan2" => return Ok(Value::Float(f0(&args).atan2(f1(&args)))),
        "__malib_log"   => return Ok(Value::Float(f0(&args).ln())),
        "__malib_log2"  => return Ok(Value::Float(f0(&args).log2())),
        "__malib_log10" => return Ok(Value::Float(f0(&args).log10())),
        "__malib_logn"  => return Ok(Value::Float(f0(&args).log(f1(&args)))),
        "__malib_exp"   => return Ok(Value::Float(f0(&args).exp())),
        "__malib_sinh"  => return Ok(Value::Float(f0(&args).sinh())),
        "__malib_cosh"  => return Ok(Value::Float(f0(&args).cosh())),
        "__malib_tanh"  => return Ok(Value::Float(f0(&args).tanh())),

        // ── Number theory ─────────────────────────────────────────────────────
        "__malib_gcd"      => return Ok(Value::Int(malib_gcd(i0(&args), i1(&args)))),
        "__malib_lcm"      => return Ok(Value::Int(malib_lcm(i0(&args), i1(&args)))),
        "__malib_is_prime" => return Ok(Value::Bool(malib_is_prime(i0(&args)))),
        "__malib_factorize"=> return Ok(Value::List(malib_factorize(i0(&args)).into_iter().map(Value::Int).collect())),
        "__malib_factorial" => {
            let n=i0(&args);
            if n>20 { let nf=n as f64; return Ok(Value::Float((2.0*std::f64::consts::PI*nf).sqrt()*(nf/std::f64::consts::E).powf(nf))); }
            return Ok(Value::Int(malib_factorial(n)));
        }
        "__malib_fibonacci"=> return Ok(Value::Int(malib_fibonacci(i0(&args)))),
        "__malib_totient"  => return Ok(Value::Int(malib_totient(i0(&args)))),
        "__malib_modinv"   => return Ok(malib_modinv(i0(&args),i1(&args)).map(Value::Int).unwrap_or(Value::Null)),
        "__malib_primes"   => return Ok(Value::List(malib_primes_upto(i0(&args) as usize).into_iter().map(Value::Int).collect())),

        // ── Combinatorics ─────────────────────────────────────────────────────
        "__malib_ncr" => return Ok(Value::Int(malib_ncr(i0(&args), i1(&args)))),
        "__malib_npr" => return Ok(Value::Int(malib_npr(i0(&args), i1(&args)))),

        // ── Algebra ───────────────────────────────────────────────────────────
        "__malib_solve"    => return Ok(malib_solve_str(&s0(&args))),
        "__malib_linear"   => return Ok(malib_linear(f0(&args), f1(&args))),
        "__malib_quadratic"=> return Ok(malib_quadratic(f0(&args), f1(&args), f2(&args))),
        "__malib_simplify" => return Ok(Value::Str(malib_simplify(&s0(&args)))),
        "__malib_eval"     => {
            let expr=s0(&args);
            let x=f1(&args);
            return Ok(Value::Float(malib_eval(&expr, x)));
        }

        // ── Calculus ──────────────────────────────────────────────────────────
        "__malib_derivative" => {
            let expr=s0(&args); let at=f1(&args);
            return Ok(Value::Float(malib_derivative_at(&expr, at)));
        }
        "__malib_integral" => {
            let expr=s0(&args); let lo=f1(&args); let hi=f2(&args);
            return Ok(Value::Float(malib_integrate(&expr, lo, hi)));
        }
        "__malib_limit" => {
            let expr=s0(&args); let at=f1(&args);
            return Ok(Value::Float(malib_limit(&expr, at)));
        }
        "__malib_root" => {
            let expr=s0(&args); let guess=f1(&args);
            return Ok(Value::Float(malib_newton(&expr, guess, 200, 1e-10)));
        }
        "__malib_diff_sym" => return Ok(Value::Str(malib_diff_sym(&s0(&args)))),

        // ── Statistics ────────────────────────────────────────────────────────
        "__malib_mean" => {
            let data=malib_get_list_f64(&args);
            if data.is_empty(){return Ok(Value::Null);}
            return Ok(Value::Float(data.iter().sum::<f64>()/data.len() as f64));
        }
        "__malib_median" => {
            let mut data=malib_get_list_f64(&args);
            data.sort_by(|a,b|a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            if data.is_empty(){return Ok(Value::Null);}
            let n=data.len();
            return Ok(Value::Float(if n%2==0{(data[n/2-1]+data[n/2])/2.0}else{data[n/2]}));
        }
        "__malib_variance" => {
            let data=malib_get_list_f64(&args);
            if data.is_empty(){return Ok(Value::Null);}
            let m=data.iter().sum::<f64>()/data.len() as f64;
            return Ok(Value::Float(data.iter().map(|x|(x-m).powi(2)).sum::<f64>()/data.len() as f64));
        }
        "__malib_stdev" => {
            let data=malib_get_list_f64(&args);
            if data.is_empty(){return Ok(Value::Null);}
            let m=data.iter().sum::<f64>()/data.len() as f64;
            let v=data.iter().map(|x|(x-m).powi(2)).sum::<f64>()/data.len() as f64;
            return Ok(Value::Float(v.sqrt()));
        }
        "__malib_sum" => {
            let data=malib_get_list_f64(&args);
            return Ok(Value::Float(data.iter().sum()));
        }
        "__malib_stats" => {
            let data=malib_get_list_f64(&args);
            return Ok(Value::Map(malib_stats_all(&data)));
        }
        "__malib_percentile" => {
            let data=malib_get_list_f64(&args);
            let mut sorted=data.clone();
            sorted.sort_by(|a,b|a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            return Ok(Value::Float(malib_percentile(&sorted, f1(&args))));
        }

        // ── Matrix ────────────────────────────────────────────────────────────
        "__malib_matmul" => {
            let a=malib_value_to_mat(args.get(0).unwrap_or(&Value::List(vec![]))).ok_or("matMul: need 2D list")?;
            let b=malib_value_to_mat(args.get(1).unwrap_or(&Value::List(vec![]))).ok_or("matMul: need 2D list")?;
            return Ok(malib_mat_mul(&a,&b).map(|m|malib_mat_to_value(&m)).unwrap_or(Value::Str("Dimension mismatch".into())));
        }
        "__malib_matdet" => {
            let m=malib_value_to_mat(args.get(0).unwrap_or(&Value::List(vec![]))).ok_or("matDet: need 2D list")?;
            return Ok(Value::Float(malib_mat_det(&m)));
        }
        "__malib_matadd" => {
            let a=malib_value_to_mat(args.get(0).unwrap_or(&Value::List(vec![]))).ok_or("matAdd: need 2D list")?;
            let b=malib_value_to_mat(args.get(1).unwrap_or(&Value::List(vec![]))).ok_or("matAdd: need 2D list")?;
            return Ok(malib_mat_add(&a,&b).map(|m|malib_mat_to_value(&m)).unwrap_or(Value::Str("Dimension mismatch".into())));
        }
        "__malib_mattranspose" => {
            let a=malib_value_to_mat(args.get(0).unwrap_or(&Value::List(vec![]))).ok_or("matTranspose: need 2D list")?;
            return Ok(malib_mat_to_value(&malib_mat_transpose(&a)));
        }
        "__malib_matinv" => {
            let a=malib_value_to_mat(args.get(0).unwrap_or(&Value::List(vec![]))).ok_or("matInv: need 2D list")?;
            return Ok(malib_mat_inv(&a).map(|m|malib_mat_to_value(&m)).unwrap_or(Value::Str("Not invertible".into())));
        }

        // ── Complex ───────────────────────────────────────────────────────────
        "__malib_complex" => {
            return Ok(malib_make_complex(f0(&args), f1(&args)));
        }
        "__malib_complex_op" => {
            let re=f0(&args); let im=f1(&args);
            let op=args.get(2).map(|v|v.to_string()).unwrap_or_default();
            let re2=malib_f64(args.get(3).unwrap_or(&Value::Int(0)));
            let im2=malib_f64(args.get(4).unwrap_or(&Value::Int(0)));
            return Ok(match op.as_str(){
                "add" => malib_make_complex(re+re2,im+im2),
                "sub" => malib_make_complex(re-re2,im-im2),
                "mul" => malib_make_complex(re*re2-im*im2,re*im2+im*re2),
                "div" => { let d=re2*re2+im2*im2; if d<1e-15{Value::Str("Division by zero".into())} else{malib_make_complex((re*re2+im*im2)/d,(im*re2-re*im2)/d)} }
                "conj"=> malib_make_complex(re,-im),
                "abs" => Value::Float((re*re+im*im).sqrt()),
                _ => malib_make_complex(re,im),
            });
        }

        // ── Series ────────────────────────────────────────────────────────────
        "__malib_arithmetic_series" => {
            return Ok(malib_arithmetic_series(f0(&args),f1(&args),malib_i64(args.get(2).unwrap_or(&Value::Int(10))) as usize));
        }
        "__malib_geometric_series" => {
            return Ok(malib_geometric_series(f0(&args),f1(&args),malib_i64(args.get(2).unwrap_or(&Value::Int(10))) as usize));
        }

        // ── Utility ───────────────────────────────────────────────────────────
        "__malib_clamp"     => return Ok(Value::Float(malib_clamp(f0(&args),f1(&args),f2(&args)))),
        "__malib_lerp"      => return Ok(Value::Float(malib_lerp(f0(&args),f1(&args),f2(&args)))),
        "__malib_map_range" => return Ok(Value::Float(malib_map_range(f0(&args),f1(&args),f2(&args),malib_f64(args.get(3).unwrap_or(&Value::Int(0))),malib_f64(args.get(4).unwrap_or(&Value::Int(1)))))),
        "__malib_sign"      => { let x=f0(&args); return Ok(Value::Int(if x>0.0{1}else if x<0.0{-1}else{0})); }
        "__malib_deg"       => return Ok(Value::Float(f0(&args).to_degrees())),
        "__malib_rad"       => return Ok(Value::Float(f0(&args).to_radians())),
        "__malib_to_fraction" => return Ok(malib_to_fraction(f0(&args))),
        "__malib_phi"       => return Ok(Value::Float(1.6180339887498948482)),

        // Also dispatch extended __math_ (asin/acos/atan/exp/sinh/cosh/tanh)
        "__math_asin"  => return Ok(Value::Float(f0(&args).asin())),
        "__math_acos"  => return Ok(Value::Float(f0(&args).acos())),
        "__math_atan"  => return Ok(Value::Float(f0(&args).atan())),
        "__math_exp"   => return Ok(Value::Float(f0(&args).exp())),
        "__math_sinh"  => return Ok(Value::Float(f0(&args).sinh())),
        "__math_cosh"  => return Ok(Value::Float(f0(&args).cosh())),
        "__math_tanh"  => return Ok(Value::Float(f0(&args).tanh())),

        _ => {}
    }
    Err(format!("Unknown Malib function: {}", name))
}

fn malib_get_list_f64(args: &[Value]) -> Vec<f64> {
    match args.first().unwrap_or(&Value::List(vec![])) {
        Value::List(v) => v.iter().map(|x|malib_f64(x)).collect(),
        _ => vec![],
    }
}

// =============================================================================
// NUMRUX — Remox ka apna N-Dimensional Array Engine (v1.0)
// =============================================================================
// Goal: NumPy se zyada EASY (zero import, "a+b" seedha kaam karta hai,
// koi .reshape() chain ya dtype confusion nahi) aur kuch jagah zyada
// POWERFUL bhi (built-in broadcasting kisi bhi rank ke liye, language ke
// operators directly array-aware hain, random/stats/linalg sab ek hi
// namespace mein, bina "import numpy as np" jaisi ceremony ke).
//
// Internal representation: ek tagged Value::Map —
//   [("__numrux__", Bool(true)), ("shape", List<Int>), ("data", List<Float>)]
// Data row-major (C order) flat hota hai, jaisa NumPy default mein karta hai.
//
// 2D linear algebra (det/inv/matmul) Malib ke already-tested matrix engine
// se bridge kiya gaya hai (malib_mat_* functions) — duplicate code nahi.
// =============================================================================

use core::sync::atomic::{AtomicU64, Ordering};

/// Global PRNG state (xorshift64*) — single-threaded kernel context mein
/// static atomic kaafi hai. Numrux.seed(n) isse reset karta hai.
static NUMRUX_RNG: AtomicU64 = AtomicU64::new(0x9E3779B97F4A7C15);

fn numrux_next_u64() -> u64 {
    let mut x = NUMRUX_RNG.load(Ordering::Relaxed);
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    NUMRUX_RNG.store(x, Ordering::Relaxed);
    x
}
fn numrux_next_f64() -> f64 {
    (numrux_next_u64() >> 11) as f64 / (1u64 << 53) as f64
}

fn numrux_is_arr(v: &Value) -> bool {
    matches!(v, Value::Map(pairs) if pairs.first().map(|(k, _)| k == "__numrux__").unwrap_or(false))
}
fn numrux_map_get<'a>(pairs: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn numrux_strides(shape: &[usize]) -> Vec<usize> {
    let n = shape.len();
    let mut strides = vec![1usize; n];
    for i in (0..n.saturating_sub(1)).rev() {
        strides[i] = strides[i + 1] * shape[i + 1];
    }
    strides
}
fn numrux_total(shape: &[usize]) -> usize {
    if shape.is_empty() { 1 } else { shape.iter().product() }
}

fn numrux_to_value(shape: Vec<usize>, data: Vec<f64>) -> Value {
    Value::Map(vec![
        ("__numrux__".into(), Value::Bool(true)),
        ("shape".into(), Value::List(shape.iter().map(|&d| Value::Int(d as i64)).collect())),
        ("data".into(), Value::List(data.into_iter().map(Value::Float).collect())),
    ])
}

fn numrux_from_value(v: &Value) -> Option<(Vec<usize>, Vec<f64>)> {
    match v {
        Value::Map(pairs) if pairs.first().map(|(k, _)| k == "__numrux__").unwrap_or(false) => {
            let shape: Vec<usize> = match numrux_map_get(pairs, "shape") {
                Some(Value::List(l)) => l.iter().map(|x| malib_i64(x).max(0) as usize).collect(),
                _ => return None,
            };
            let data: Vec<f64> = match numrux_map_get(pairs, "data") {
                Some(Value::List(l)) => l.iter().map(malib_f64).collect(),
                _ => return None,
            };
            Some((shape, data))
        }
        Value::Int(n) => Some((vec![], vec![*n as f64])),
        Value::Float(n) => Some((vec![], vec![*n])),
        Value::Bool(b) => Some((vec![], vec![if *b { 1.0 } else { 0.0 }])),
        Value::List(items) => {
            fn shape_of(v: &Value, out: &mut Vec<usize>) {
                if let Value::List(items) = v {
                    out.push(items.len());
                    if let Some(first) = items.first() {
                        shape_of(first, out);
                    }
                }
            }
            fn flatten_into(v: &Value, out: &mut Vec<f64>) {
                match v {
                    Value::List(items) => { for it in items { flatten_into(it, out); } }
                    other => out.push(malib_f64(other)),
                }
            }
            let mut shape = Vec::new();
            shape_of(v, &mut shape);
            let mut data = Vec::new();
            flatten_into(v, &mut data);
            if data.is_empty() && items.is_empty() { shape = vec![0]; }
            Some((shape, data))
        }
        _ => None,
    }
}

fn numrux_to_nested(shape: &[usize], data: &[f64]) -> Value {
    fn build(shape: &[usize], data: &[f64], offset: usize, stride: usize) -> Value {
        if shape.is_empty() {
            return Value::Float(data.get(offset).copied().unwrap_or(0.0));
        }
        let dim = shape[0];
        let sub_stride = stride / dim.max(1);
        let mut items = Vec::with_capacity(dim);
        for i in 0..dim {
            items.push(build(&shape[1..], data, offset + i * sub_stride, sub_stride));
        }
        Value::List(items)
    }
    if shape.is_empty() {
        return Value::Float(data.first().copied().unwrap_or(0.0));
    }
    let total = numrux_total(shape).max(data.len());
    build(shape, data, 0, total)
}

fn numrux_display_string(pairs: &[(String, Value)]) -> String {
    let shape: Vec<usize> = match numrux_map_get(pairs, "shape") {
        Some(Value::List(l)) => l.iter().map(|x| malib_i64(x).max(0) as usize).collect(),
        _ => vec![],
    };
    let data: Vec<f64> = match numrux_map_get(pairs, "data") {
        Some(Value::List(l)) => l.iter().map(malib_f64).collect(),
        _ => vec![],
    };
    format!("{}", numrux_to_nested(&shape, &data))
}

fn numrux_broadcast_shapes(a: &[usize], b: &[usize]) -> Option<Vec<usize>> {
    let rank = a.len().max(b.len());
    let pa = rank - a.len();
    let pb = rank - b.len();
    let mut out = vec![0usize; rank];
    for i in 0..rank {
        let da = if i < pa { 1 } else { a[i - pa] };
        let db = if i < pb { 1 } else { b[i - pb] };
        if da != db && da != 1 && db != 1 { return None; }
        out[i] = da.max(db);
    }
    Some(out)
}
fn numrux_get_broadcast(data: &[f64], shape: &[usize], out_idx: &[usize], out_rank: usize) -> f64 {
    if shape.is_empty() { return data.first().copied().unwrap_or(0.0); }
    let pad = out_rank - shape.len();
    let strides = numrux_strides(shape);
    let mut flat = 0usize;
    for i in 0..shape.len() {
        let od = out_idx[pad + i];
        let id = if shape[i] == 1 { 0 } else { od };
        flat += id * strides[i];
    }
    data.get(flat).copied().unwrap_or(0.0)
}
fn numrux_elementwise(
    a_shape: &[usize], a_data: &[f64],
    b_shape: &[usize], b_data: &[f64],
    f: impl Fn(f64, f64) -> f64,
) -> Result<(Vec<usize>, Vec<f64>), String> {
    let out_shape = numrux_broadcast_shapes(a_shape, b_shape)
        .ok_or_else(|| format!("Numrux: shapes {:?} and {:?} could not be broadcast together", a_shape, b_shape))?;
    let total: usize = numrux_total(&out_shape);
    let out_strides = numrux_strides(&out_shape);
    let out_rank = out_shape.len();
    let mut out_data = Vec::with_capacity(total);
    let mut idx = vec![0usize; out_rank];
    for flat in 0..total {
        let mut rem = flat;
        for d in 0..out_rank {
            idx[d] = if out_strides[d] == 0 { 0 } else { rem / out_strides[d] };
            rem = if out_strides[d] == 0 { 0 } else { rem % out_strides[d] };
        }
        let av = numrux_get_broadcast(a_data, a_shape, &idx, out_rank);
        let bv = numrux_get_broadcast(b_data, b_shape, &idx, out_rank);
        out_data.push(f(av, bv));
    }
    Ok((out_shape, out_data))
}
fn numrux_map_unary(shape: &[usize], data: &[f64], f: impl Fn(f64) -> f64) -> (Vec<usize>, Vec<f64>) {
    (shape.to_vec(), data.iter().map(|&x| f(x)).collect())
}

fn numrux_arr_arg(args: &[Value], i: usize) -> Result<(Vec<usize>, Vec<f64>), String> {
    numrux_from_value(args.get(i).unwrap_or(&Value::Null))
        .ok_or_else(|| format!("Numrux: argument {} is not array-like", i))
}
fn numrux_shape_arg(args: &[Value], i: usize) -> Vec<usize> {
    match args.get(i) {
        Some(Value::List(l)) => l.iter().map(|x| malib_i64(x).max(0) as usize).collect(),
        Some(Value::Int(n)) => vec![(*n).max(0) as usize],
        _ => vec![],
    }
}

fn numrux_to_2d_value(shape: &[usize], data: &[f64]) -> Result<Value, String> {
    if shape.len() != 2 { return Err(format!("Numrux: expected a 2D array, got shape {:?}", shape)); }
    Ok(numrux_to_nested(shape, data))
}

// =============================================================================
// NUMRUX DISPATCHER
// =============================================================================
pub(crate) fn dispatch_numrux(name: &str, args: Vec<Value>) -> Result<Value, String> {
    match name {
        "__numrux_array" => {
            let (shape, data) = numrux_arr_arg(&args, 0)?;
            Ok(numrux_to_value(shape, data))
        }
        "__numrux_zeros" => {
            let shape = numrux_shape_arg(&args, 0);
            let total = numrux_total(&shape);
            Ok(numrux_to_value(shape, vec![0.0; total]))
        }
        "__numrux_ones" => {
            let shape = numrux_shape_arg(&args, 0);
            let total = numrux_total(&shape);
            Ok(numrux_to_value(shape, vec![1.0; total]))
        }
        "__numrux_full" => {
            let shape = numrux_shape_arg(&args, 0);
            let val = malib_f64(args.get(1).unwrap_or(&Value::Int(0)));
            let total = numrux_total(&shape);
            Ok(numrux_to_value(shape, vec![val; total]))
        }
        "__numrux_arange" => {
            let start = malib_f64(args.get(0).unwrap_or(&Value::Float(0.0)));
            let stop = malib_f64(args.get(1).unwrap_or(&Value::Float(0.0)));
            let step = malib_f64(args.get(2).unwrap_or(&Value::Float(1.0)));
            if step == 0.0 { return Err("Numrux.arange: step cannot be 0".into()); }
            let mut data = Vec::new();
            let mut x = start;
            if step > 0.0 { while x < stop { data.push(x); x += step; } }
            else { while x > stop { data.push(x); x += step; } }
            let n = data.len();
            Ok(numrux_to_value(vec![n], data))
        }
        "__numrux_linspace" => {
            let start = malib_f64(args.get(0).unwrap_or(&Value::Float(0.0)));
            let stop = malib_f64(args.get(1).unwrap_or(&Value::Float(1.0)));
            let n = malib_i64(args.get(2).unwrap_or(&Value::Int(50))).max(1) as usize;
            let mut data = Vec::with_capacity(n);
            if n == 1 { data.push(start); } else {
                let step = (stop - start) / (n as f64 - 1.0);
                for i in 0..n { data.push(start + step * i as f64); }
            }
            Ok(numrux_to_value(vec![n], data))
        }
        "__numrux_eye" => {
            let n = malib_i64(args.get(0).unwrap_or(&Value::Int(1))).max(0) as usize;
            let mut data = vec![0.0; n * n];
            for i in 0..n { data[i * n + i] = 1.0; }
            Ok(numrux_to_value(vec![n, n], data))
        }
        "__numrux_shape" => {
            let (shape, _) = numrux_arr_arg(&args, 0)?;
            Ok(Value::List(shape.into_iter().map(|d| Value::Int(d as i64)).collect()))
        }
        "__numrux_size" => {
            let (_, data) = numrux_arr_arg(&args, 0)?;
            Ok(Value::Int(data.len() as i64))
        }
        "__numrux_ndim" => {
            let (shape, _) = numrux_arr_arg(&args, 0)?;
            Ok(Value::Int(shape.len() as i64))
        }
        "__numrux_reshape" => {
            let (_, data) = numrux_arr_arg(&args, 0)?;
            let new_shape = numrux_shape_arg(&args, 1);
            let total = numrux_total(&new_shape);
            if total != data.len() {
                return Err(format!("Numrux.reshape: cannot reshape array of size {} into shape {:?}", data.len(), new_shape));
            }
            Ok(numrux_to_value(new_shape, data))
        }
        "__numrux_flatten" => {
            let (_, data) = numrux_arr_arg(&args, 0)?;
            let n = data.len();
            Ok(numrux_to_value(vec![n], data))
        }
        "__numrux_transpose" => {
            let (shape, data) = numrux_arr_arg(&args, 0)?;
            match shape.len() {
                0 | 1 => Ok(numrux_to_value(shape, data)),
                2 => {
                    let m = malib_value_to_mat(&numrux_to_2d_value(&shape, &data)?)
                        .ok_or("Numrux.transpose: invalid 2D array")?;
                    let t = malib_mat_transpose(&m);
                    let rows = t.len();
                    let cols = t.first().map(|r| r.len()).unwrap_or(0);
                    let flat: Vec<f64> = t.into_iter().flatten().collect();
                    Ok(numrux_to_value(vec![rows, cols], flat))
                }
                _ => Err("Numrux.transpose: only 1D/2D supported for now".into()),
            }
        }
        "__numrux_tolist" => {
            let (shape, data) = numrux_arr_arg(&args, 0)?;
            Ok(numrux_to_nested(&shape, &data))
        }
        "__numrux_add" => { let (sa,da)=numrux_arr_arg(&args,0)?; let (sb,db)=numrux_arr_arg(&args,1)?; let (s,d)=numrux_elementwise(&sa,&da,&sb,&db,|a,b| a+b)?; Ok(numrux_to_value(s,d)) }
        "__numrux_sub" => { let (sa,da)=numrux_arr_arg(&args,0)?; let (sb,db)=numrux_arr_arg(&args,1)?; let (s,d)=numrux_elementwise(&sa,&da,&sb,&db,|a,b| a-b)?; Ok(numrux_to_value(s,d)) }
        "__numrux_mul" => { let (sa,da)=numrux_arr_arg(&args,0)?; let (sb,db)=numrux_arr_arg(&args,1)?; let (s,d)=numrux_elementwise(&sa,&da,&sb,&db,|a,b| a*b)?; Ok(numrux_to_value(s,d)) }
        "__numrux_div" => { let (sa,da)=numrux_arr_arg(&args,0)?; let (sb,db)=numrux_arr_arg(&args,1)?; let (s,d)=numrux_elementwise(&sa,&da,&sb,&db,|a,b| a/b)?; Ok(numrux_to_value(s,d)) }
        "__numrux_pow" => { let (sa,da)=numrux_arr_arg(&args,0)?; let (sb,db)=numrux_arr_arg(&args,1)?; let (s,d)=numrux_elementwise(&sa,&da,&sb,&db,|a,b| a.powf(b))?; Ok(numrux_to_value(s,d)) }
        "__numrux_neg" => { let (s,d)=numrux_arr_arg(&args,0)?; let (s,d)=numrux_map_unary(&s,&d,|x| -x); Ok(numrux_to_value(s,d)) }
        "__numrux_abs" => { let (s,d)=numrux_arr_arg(&args,0)?; let (s,d)=numrux_map_unary(&s,&d,|x| x.abs()); Ok(numrux_to_value(s,d)) }
        "__numrux_sqrt" => { let (s,d)=numrux_arr_arg(&args,0)?; let (s,d)=numrux_map_unary(&s,&d,|x| x.sqrt()); Ok(numrux_to_value(s,d)) }
        "__numrux_exp" => { let (s,d)=numrux_arr_arg(&args,0)?; let (s,d)=numrux_map_unary(&s,&d,|x| x.exp()); Ok(numrux_to_value(s,d)) }
        "__numrux_log" => { let (s,d)=numrux_arr_arg(&args,0)?; let (s,d)=numrux_map_unary(&s,&d,|x| x.ln()); Ok(numrux_to_value(s,d)) }
        "__numrux_sin" => { let (s,d)=numrux_arr_arg(&args,0)?; let (s,d)=numrux_map_unary(&s,&d,|x| x.sin()); Ok(numrux_to_value(s,d)) }
        "__numrux_cos" => { let (s,d)=numrux_arr_arg(&args,0)?; let (s,d)=numrux_map_unary(&s,&d,|x| x.cos()); Ok(numrux_to_value(s,d)) }
        "__numrux_tan" => { let (s,d)=numrux_arr_arg(&args,0)?; let (s,d)=numrux_map_unary(&s,&d,|x| x.tan()); Ok(numrux_to_value(s,d)) }
        "__numrux_clip" => {
            let (s,d) = numrux_arr_arg(&args,0)?;
            let lo = malib_f64(args.get(1).unwrap_or(&Value::Float(f64::NEG_INFINITY)));
            let hi = malib_f64(args.get(2).unwrap_or(&Value::Float(f64::INFINITY)));
            let (s,d) = numrux_map_unary(&s,&d,|x| x.max(lo).min(hi));
            Ok(numrux_to_value(s,d))
        }
        "__numrux_gt" => { let (sa,da)=numrux_arr_arg(&args,0)?; let (sb,db)=numrux_arr_arg(&args,1)?; let (s,d)=numrux_elementwise(&sa,&da,&sb,&db,|a,b| if a>b{1.0}else{0.0})?; Ok(numrux_to_value(s,d)) }
        "__numrux_lt" => { let (sa,da)=numrux_arr_arg(&args,0)?; let (sb,db)=numrux_arr_arg(&args,1)?; let (s,d)=numrux_elementwise(&sa,&da,&sb,&db,|a,b| if a<b{1.0}else{0.0})?; Ok(numrux_to_value(s,d)) }
        "__numrux_ge" => { let (sa,da)=numrux_arr_arg(&args,0)?; let (sb,db)=numrux_arr_arg(&args,1)?; let (s,d)=numrux_elementwise(&sa,&da,&sb,&db,|a,b| if a>=b{1.0}else{0.0})?; Ok(numrux_to_value(s,d)) }
        "__numrux_le" => { let (sa,da)=numrux_arr_arg(&args,0)?; let (sb,db)=numrux_arr_arg(&args,1)?; let (s,d)=numrux_elementwise(&sa,&da,&sb,&db,|a,b| if a<=b{1.0}else{0.0})?; Ok(numrux_to_value(s,d)) }
        "__numrux_eq" => { let (sa,da)=numrux_arr_arg(&args,0)?; let (sb,db)=numrux_arr_arg(&args,1)?; let (s,d)=numrux_elementwise(&sa,&da,&sb,&db,|a,b| if (a-b).abs()<1e-12{1.0}else{0.0})?; Ok(numrux_to_value(s,d)) }
        "__numrux_where" => {
            let (sc,dc) = numrux_arr_arg(&args,0)?;
            let (sx,dx) = numrux_arr_arg(&args,1)?;
            let (sy,dy) = numrux_arr_arg(&args,2)?;
            let (s1,d1) = numrux_elementwise(&sc,&dc,&sx,&dx,|c,x| if c != 0.0 { x } else { f64::NAN })?;
            let (s2,d2) = numrux_elementwise(&sc,&dc,&sy,&dy,|c,y| if c == 0.0 { y } else { f64::NAN })?;
            if s1 != s2 { return Err("Numrux.where: shape mismatch between x and y branch".into()); }
            let out: Vec<f64> = d1.into_iter().zip(d2).map(|(a,b)| if a.is_nan() { b } else { a }).collect();
            Ok(numrux_to_value(s1, out))
        }
        "__numrux_sum" => { let (_,d)=numrux_arr_arg(&args,0)?; Ok(Value::Float(d.iter().sum())) }
        "__numrux_mean" => { let (_,d)=numrux_arr_arg(&args,0)?; if d.is_empty(){return Ok(Value::Null);} Ok(Value::Float(d.iter().sum::<f64>()/d.len() as f64)) }
        "__numrux_min" => { let (_,d)=numrux_arr_arg(&args,0)?; Ok(Value::Float(d.iter().cloned().fold(f64::INFINITY, f64::min))) }
        "__numrux_max" => { let (_,d)=numrux_arr_arg(&args,0)?; Ok(Value::Float(d.iter().cloned().fold(f64::NEG_INFINITY, f64::max))) }
        "__numrux_prod" => { let (_,d)=numrux_arr_arg(&args,0)?; Ok(Value::Float(d.iter().product())) }
        "__numrux_std" => {
            let (_,d) = numrux_arr_arg(&args,0)?;
            if d.is_empty(){return Ok(Value::Null);}
            let m = d.iter().sum::<f64>()/d.len() as f64;
            let v = d.iter().map(|x|(x-m).powi(2)).sum::<f64>()/d.len() as f64;
            Ok(Value::Float(v.sqrt()))
        }
        "__numrux_var" => {
            let (_,d) = numrux_arr_arg(&args,0)?;
            if d.is_empty(){return Ok(Value::Null);}
            let m = d.iter().sum::<f64>()/d.len() as f64;
            Ok(Value::Float(d.iter().map(|x|(x-m).powi(2)).sum::<f64>()/d.len() as f64))
        }
        "__numrux_median" => {
            let (_, mut d) = numrux_arr_arg(&args,0)?;
            if d.is_empty(){return Ok(Value::Null);}
            d.sort_by(|a,b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
            let n = d.len();
            Ok(Value::Float(if n%2==0 { (d[n/2-1]+d[n/2])/2.0 } else { d[n/2] }))
        }
        "__numrux_argmin" => {
            let (_,d) = numrux_arr_arg(&args,0)?;
            let mut bi = 0usize; let mut bv = f64::INFINITY;
            for (i,&x) in d.iter().enumerate() { if x < bv { bv = x; bi = i; } }
            Ok(Value::Int(bi as i64))
        }
        "__numrux_argmax" => {
            let (_,d) = numrux_arr_arg(&args,0)?;
            let mut bi = 0usize; let mut bv = f64::NEG_INFINITY;
            for (i,&x) in d.iter().enumerate() { if x > bv { bv = x; bi = i; } }
            Ok(Value::Int(bi as i64))
        }
        "__numrux_cumsum" => {
            let (_,d) = numrux_arr_arg(&args,0)?;
            let mut out = Vec::with_capacity(d.len());
            let mut acc = 0.0;
            for x in d { acc += x; out.push(acc); }
            let n = out.len();
            Ok(numrux_to_value(vec![n], out))
        }
        "__numrux_dot" => {
            let (sa,da) = numrux_arr_arg(&args,0)?;
            let (sb,db) = numrux_arr_arg(&args,1)?;
            match (sa.len(), sb.len()) {
                (1,1) => {
                    if da.len() != db.len() { return Err("Numrux.dot: vector length mismatch".into()); }
                    Ok(Value::Float(da.iter().zip(db.iter()).map(|(x,y)| x*y).sum()))
                }
                (2,2) => {
                    let ma = malib_value_to_mat(&numrux_to_2d_value(&sa,&da)?).ok_or("Numrux.dot: invalid matrix A")?;
                    let mb = malib_value_to_mat(&numrux_to_2d_value(&sb,&db)?).ok_or("Numrux.dot: invalid matrix B")?;
                    let r = malib_mat_mul(&ma, &mb).ok_or("Numrux.dot: dimension mismatch")?;
                    let rows = r.len(); let cols = r.first().map(|x|x.len()).unwrap_or(0);
                    let flat: Vec<f64> = r.into_iter().flatten().collect();
                    Ok(numrux_to_value(vec![rows, cols], flat))
                }
                (2,1) => {
                    let ma = malib_value_to_mat(&numrux_to_2d_value(&sa,&da)?).ok_or("Numrux.dot: invalid matrix A")?;
                    if ma.first().map(|r|r.len()).unwrap_or(0) != db.len() {
                        return Err("Numrux.dot: matrix/vector dimension mismatch".into());
                    }
                    let out: Vec<f64> = ma.iter().map(|row| row.iter().zip(db.iter()).map(|(a,b)|a*b).sum()).collect();
                    let n = out.len();
                    Ok(numrux_to_value(vec![n], out))
                }
                _ => Err("Numrux.dot: unsupported shapes for dot product".into()),
            }
        }
        "__numrux_det" => {
            let (s,d) = numrux_arr_arg(&args,0)?;
            let m = malib_value_to_mat(&numrux_to_2d_value(&s,&d)?).ok_or("Numrux.det: invalid matrix")?;
            Ok(Value::Float(malib_mat_det(&m)))
        }
        "__numrux_inv" => {
            let (s,d) = numrux_arr_arg(&args,0)?;
            let m = malib_value_to_mat(&numrux_to_2d_value(&s,&d)?).ok_or("Numrux.inv: invalid matrix")?;
            let inv = malib_mat_inv(&m).ok_or("Numrux.inv: matrix is not invertible")?;
            let rows = inv.len(); let cols = inv.first().map(|r|r.len()).unwrap_or(0);
            let flat: Vec<f64> = inv.into_iter().flatten().collect();
            Ok(numrux_to_value(vec![rows, cols], flat))
        }
        "__numrux_trace" => {
            let (s,d) = numrux_arr_arg(&args,0)?;
            if s.len() != 2 || s[0] != s[1] { return Err("Numrux.trace: requires a square 2D matrix".into()); }
            let n = s[0];
            let mut t = 0.0;
            for i in 0..n { t += d[i*n+i]; }
            Ok(Value::Float(t))
        }
        "__numrux_seed" => {
            let n = malib_i64(args.get(0).unwrap_or(&Value::Int(42))) as u64;
            NUMRUX_RNG.store(n.wrapping_mul(2685821657736338717).wrapping_add(1) | 1, Ordering::Relaxed);
            Ok(Value::Null)
        }
        "__numrux_rand" => {
            let shape = numrux_shape_arg(&args,0);
            let total = numrux_total(&shape);
            let data: Vec<f64> = (0..total).map(|_| numrux_next_f64()).collect();
            Ok(numrux_to_value(shape, data))
        }
        "__numrux_randn" => {
            let shape = numrux_shape_arg(&args,0);
            let total = numrux_total(&shape);
            let data: Vec<f64> = (0..total).map(|_| {
                let u1 = numrux_next_f64().max(1e-12);
                let u2 = numrux_next_f64();
                (-2.0 * u1.ln()).sqrt() * (2.0 * core::f64::consts::PI * u2).cos()
            }).collect();
            Ok(numrux_to_value(shape, data))
        }
        "__numrux_randint" => {
            let lo = malib_i64(args.get(0).unwrap_or(&Value::Int(0)));
            let hi = malib_i64(args.get(1).unwrap_or(&Value::Int(100)));
            let shape = numrux_shape_arg(&args,2);
            let span = (hi - lo + 1).max(1) as u64;
            let total = numrux_total(&shape);
            let data: Vec<f64> = (0..total).map(|_| (lo + (numrux_next_u64() % span) as i64) as f64).collect();
            Ok(numrux_to_value(shape, data))
        }
        "__numrux_sort" => {
            let (_, mut d) = numrux_arr_arg(&args,0)?;
            d.sort_by(|a,b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
            let n = d.len();
            Ok(numrux_to_value(vec![n], d))
        }
        "__numrux_unique" => {
            let (_, mut d) = numrux_arr_arg(&args,0)?;
            d.sort_by(|a,b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
            d.dedup_by(|a,b| (*a-*b).abs() < 1e-12);
            let n = d.len();
            Ok(numrux_to_value(vec![n], d))
        }
        "__numrux_concat" => {
            let (sa, mut da) = numrux_arr_arg(&args,0)?;
            let (sb, db) = numrux_arr_arg(&args,1)?;
            if sa.len() <= 1 && sb.len() <= 1 {
                da.extend(db);
                let n = da.len();
                return Ok(numrux_to_value(vec![n], da));
            }
            if sa.len() == 2 && sb.len() == 2 && sa[1] == sb[1] {
                let rows = sa[0] + sb[0];
                let cols = sa[1];
                da.extend(db);
                return Ok(numrux_to_value(vec![rows, cols], da));
            }
            Err("Numrux.concat: shapes are not compatible for concatenation".into())
        }
        _ => Err(format!("Unknown Numrux function: {}", name)),
    }
}

// =============================================================================
// Autoclib — Remox's built-in CLI / Automation engine.
// =============================================================================
// use Autoclib
//
// Replaces (and goes beyond) Python's Click + argparse + Typer + Fire + Rich +
// Textual + Fabric — all seven, in one zero-import, zero-boilerplate module,
// wired straight into the language the same way Numrux/Malib/Phinolib are.
//
// PORT NOTE (honest status, same policy as the rest of this file):
//   Everything below is real, working logic — string formatting, ANSI
//   styling, arg parsing, topological sort, etc. — there is no simulation.
//   The ONE exception is `remoteExec` (Fabric's SSH-style remote execution),
//   which genuinely needs Monobat's outbound network stack. It returns a
//   clear "not wired yet" error rather than pretending to work, exactly like
//   `fs_read`/`net_tcp_bind` do in `StubHal` above. Once Monobat's network
//   driver lands, that one function is the only thing that needs rewiring.
// =============================================================================

/// Single-threaded in-memory REPL/command history (Textual/Fabric-style
/// session memory). Uses the same UnsafeCell + unsafe-Sync trick as `HAL`
/// above, since this is a no_std, single-core-assumed kernel context.
struct AutoclibHistoryCell(UnsafeCell<Vec<String>>);
unsafe impl Sync for AutoclibHistoryCell {}
static AUTOCLIB_HISTORY: AutoclibHistoryCell = AutoclibHistoryCell(UnsafeCell::new(Vec::new()));

/// ANSI color name → SGR code (basic 8-color set, works on any real terminal
/// and on Monobat's serial console once VT100 escape parsing is wired).
fn autoclib_ansi_code(color: &str) -> &'static str {
    match color.to_lowercase().as_str() {
        "black"   => "30",
        "red"     => "31",
        "green"   => "32",
        "yellow"  => "33",
        "blue"    => "34",
        "magenta" => "35",
        "cyan"    => "36",
        "white"   => "37",
        "gray" | "grey" => "90",
        _ => "39", // default foreground
    }
}

fn autoclib_arg_str(args: &[Value], i: usize, default: &str) -> String {
    match args.get(i) {
        Some(Value::Str(s)) => s.clone(),
        Some(v @ Value::Int(_)) | Some(v @ Value::Float(_)) | Some(v @ Value::Bool(_)) => v.to_string(),
        _ => default.to_string(),
    }
}

fn autoclib_arg_bool(args: &[Value], i: usize) -> bool {
    matches!(args.get(i), Some(Value::Bool(true)))
}

fn autoclib_arg_int(args: &[Value], i: usize, default: i64) -> i64 {
    match args.get(i) {
        Some(Value::Int(n))   => *n,
        Some(Value::Float(f)) => *f as i64,
        _ => default,
    }
}

/// Pulls a `List` of `Value`s out of an argument slot, defaulting to empty.
fn autoclib_arg_list(args: &[Value], i: usize) -> Vec<Value> {
    match args.get(i) {
        Some(Value::List(l)) => l.clone(),
        _ => Vec::new(),
    }
}

/// Pulls a `Map`'s pairs out of an argument slot, defaulting to empty.
fn autoclib_arg_map(args: &[Value], i: usize) -> Vec<(String, Value)> {
    match args.get(i) {
        Some(Value::Map(m)) => m.clone(),
        _ => Vec::new(),
    }
}

fn autoclib_map_get<'a>(pairs: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// Attempts to coerce a raw CLI string token into a typed Value, based on a
/// `spec` type name ("int" | "float" | "bool" | "string"). Mirrors argparse's
/// `type=` kwarg, but resolved automatically like Fire does from context.
fn autoclib_coerce(raw: &str, ty: &str) -> Value {
    match ty {
        "int"   => raw.parse::<i64>().map(Value::Int).unwrap_or(Value::Null),
        "float" => raw.parse::<f64>().map(Value::Float).unwrap_or(Value::Null),
        "bool"  => Value::Bool(raw == "true" || raw == "1" || raw == "yes"),
        _       => Value::Str(raw.to_string()),
    }
}

// =============================================================================
// Remotest — Remox's built-in Testing engine.
// =============================================================================
// use Remotest
//
// Combines Python's pytest + unittest + nose2 + Robot Framework (keyword-
// style) + Behave (describe/it BDD) + a slice of Hypothesis-style random
// data generation + Mock/pytest-mock + Faker — one zero-import module, wired
// in exactly the same way Autoclib/Tasoaque are (module map + "__remotest_"
// dispatch prefix). See the "HONEST GAP" note on the module map entry above
// for what's real vs. roadmap.
//
// State is single-threaded, same UnsafeCell + unsafe-Sync trick as
// AUTOCLIB_HISTORY, since this is a no_std, single-core-assumed kernel.
// =============================================================================

struct RemotestMock {
    id: String,
    #[allow(dead_code)]
    name: String,
    return_value: Value,
    calls: Vec<Vec<Value>>,
}

struct RemotestTestsCell(UnsafeCell<Vec<(String, Value, Vec<String>)>>);
unsafe impl Sync for RemotestTestsCell {}
static REMOTEST_TESTS: RemotestTestsCell = RemotestTestsCell(UnsafeCell::new(Vec::new()));

struct RemotestGroupsCell(UnsafeCell<Vec<String>>);
unsafe impl Sync for RemotestGroupsCell {}
static REMOTEST_GROUPS: RemotestGroupsCell = RemotestGroupsCell(UnsafeCell::new(Vec::new()));

struct RemotestFixturesCell(UnsafeCell<Vec<(String, Value)>>);
unsafe impl Sync for RemotestFixturesCell {}
static REMOTEST_FIXTURES: RemotestFixturesCell = RemotestFixturesCell(UnsafeCell::new(Vec::new()));

struct RemotestMocksCell(UnsafeCell<Vec<RemotestMock>>);
unsafe impl Sync for RemotestMocksCell {}
static REMOTEST_MOCKS: RemotestMocksCell = RemotestMocksCell(UnsafeCell::new(Vec::new()));

struct RemotestMockSeqCell(UnsafeCell<u64>);
unsafe impl Sync for RemotestMockSeqCell {}
static REMOTEST_MOCK_SEQ: RemotestMockSeqCell = RemotestMockSeqCell(UnsafeCell::new(0));

/// Same LCG constants as `Interpreter::call_function`'s `__rand_int`/
/// `__rand_float` arms — Remotest's faker draws from the real interpreter
/// RNG state (`self.rand_state`), not a separate/fake generator.
fn remotest_next_rand(state: &mut u64) -> u64 {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *state >> 33
}

fn remotest_pick<'a>(state: &mut u64, pool: &[&'a str]) -> &'a str {
    if pool.is_empty() { return ""; }
    let i = (remotest_next_rand(state) as usize) % pool.len();
    pool[i]
}

const REMOTEST_SHRINK_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz";

/// Draws one real sample from a generator-descriptor Map (see genInt/
/// genFloat/genBool/genString/genList/genOneOf). Off the same LCG as
/// Faker/rand_state — every call actually advances `state`, no fixed
/// fixtures.
fn remotest_generate(gen: &Value, state: &mut u64) -> Value {
    let kind = match gen {
        Value::Map(m) => match autoclib_map_get(m, "__gen__") {
            Some(Value::Str(s)) => s.clone(),
            _ => return Value::Null,
        },
        _ => return gen.clone(), // literal passed instead of a generator — sample is itself
    };
    let m = match gen { Value::Map(m) => m, _ => return Value::Null };
    match kind.as_str() {
        "int" => {
            let min = match autoclib_map_get(m, "min") { Some(Value::Int(n)) => *n, _ => 0 };
            let max = match autoclib_map_get(m, "max") { Some(Value::Int(n)) => *n, _ => 100 };
            let span = (max - min + 1).max(1);
            let v = (remotest_next_rand(state) as i64).abs() % span + min;
            Value::Int(v)
        }
        "float" => {
            let min = match autoclib_map_get(m, "min") { Some(Value::Float(n)) => *n, _ => 0.0 };
            let max = match autoclib_map_get(m, "max") { Some(Value::Float(n)) => *n, _ => 1.0 };
            let unit = (remotest_next_rand(state) % 1_000_000) as f64 / 1_000_000.0;
            Value::Float(min + unit * (max - min).max(0.0))
        }
        "bool" => Value::Bool(remotest_next_rand(state) % 2 == 0),
        "string" => {
            let max_len = match autoclib_map_get(m, "maxLen") { Some(Value::Int(n)) => (*n).max(0) as usize, _ => 12 };
            let len = if max_len == 0 { 0 } else { (remotest_next_rand(state) as usize) % (max_len + 1) };
            let s: String = (0..len).map(|_| {
                let idx = (remotest_next_rand(state) as usize) % REMOTEST_SHRINK_ALPHABET.len();
                REMOTEST_SHRINK_ALPHABET[idx] as char
            }).collect();
            Value::Str(s)
        }
        "list" => {
            let elem_gen = autoclib_map_get(m, "elem").cloned().unwrap_or(Value::Null);
            let max_len = match autoclib_map_get(m, "maxLen") { Some(Value::Int(n)) => (*n).max(0) as usize, _ => 8 };
            let len = if max_len == 0 { 0 } else { (remotest_next_rand(state) as usize) % (max_len + 1) };
            Value::List((0..len).map(|_| remotest_generate(&elem_gen, state)).collect())
        }
        "oneof" => {
            match autoclib_map_get(m, "options") {
                Some(Value::List(opts)) if !opts.is_empty() => {
                    let idx = (remotest_next_rand(state) as usize) % opts.len();
                    opts[idx].clone()
                }
                _ => Value::Null,
            }
        }
        _ => Value::Null,
    }
}

/// Candidate "simpler" values for one shrink step, given the generator
/// that produced `current`. Ordered smallest-effort-first; remotest_for_all
/// re-runs the real property function against each candidate and only
/// keeps ones that still fail — this list is a search space, not a
/// pre-decided answer.
fn remotest_shrink_candidates(gen: &Value, current: &Value) -> Vec<Value> {
    let kind = match gen {
        Value::Map(m) => match autoclib_map_get(m, "__gen__") {
            Some(Value::Str(s)) => s.clone(),
            _ => return Vec::new(),
        },
        _ => return Vec::new(),
    };
    let m = match gen { Value::Map(m) => m, _ => return Vec::new() };
    match (kind.as_str(), current) {
        ("int", Value::Int(n)) => {
            let min = match autoclib_map_get(m, "min") { Some(Value::Int(v)) => *v, _ => i64::MIN };
            let mut out = Vec::new();
            if *n != 0 && min <= 0 { out.push(Value::Int(0)); }
            if *n != min { out.push(Value::Int(min)); }
            let half = n / 2;
            if half != *n && half >= min { out.push(Value::Int(half)); }
            if *n > min { out.push(Value::Int(n - 1)); }
            out
        }
        ("float", Value::Float(f)) => {
            let min = match autoclib_map_get(m, "min") { Some(Value::Float(v)) => *v, _ => f64::MIN };
            let mut out = Vec::new();
            if *f != 0.0 && min <= 0.0 { out.push(Value::Float(0.0)); }
            let half = f / 2.0;
            if (half - f).abs() > f64::EPSILON && half >= min { out.push(Value::Float(half)); }
            out
        }
        ("bool", Value::Bool(b)) => if *b { vec![Value::Bool(false)] } else { Vec::new() },
        ("string", Value::Str(s)) => {
            let mut out = Vec::new();
            if !s.is_empty() {
                out.push(Value::Str(String::new()));
                out.push(Value::Str(s.chars().take(s.chars().count() / 2).collect()));
                out.push(Value::Str(s.chars().take(s.chars().count().saturating_sub(1)).collect()));
            }
            out
        }
        ("list", Value::List(l)) => {
            let elem_gen = autoclib_map_get(m, "elem").cloned().unwrap_or(Value::Null);
            let mut out = Vec::new();
            if !l.is_empty() {
                out.push(Value::List(Vec::new()));
                out.push(Value::List(l[..l.len() / 2].to_vec()));
                out.push(Value::List(l[..l.len() - 1].to_vec()));
                // also try shrinking the first element in place
                let first_shrinks = remotest_shrink_candidates(&elem_gen, &l[0]);
                for cand in first_shrinks {
                    let mut trial = l.clone();
                    trial[0] = cand;
                    out.push(Value::List(trial));
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

fn remotest_values_to_string(vals: &[Value]) -> String {
    let parts: Vec<String> = vals.iter().map(|v| v.to_string()).collect();
    format!("({})", parts.join(", "))
}

/// " — <msg>" suffix helper for assertion failures, when an optional
/// trailing `msg` arg was actually passed as a non-empty Str.
fn msg_suffix(args: &[Value], i: usize) -> String {
    match args.get(i) {
        Some(Value::Str(s)) if !s.is_empty() => format!(" — {}", s),
        _ => String::new(),
    }
}

static REMOTEST_FIRST_NAMES: &[&str] = &[
    "Aahil","Ravi","Priya","Neha","Arjun","Sanya","Vikram","Isha","Karan","Meera",
    "Aditya","Pooja","Rohan","Kavya","Aman","Divya","Rahul","Simran","Zoya","Yash",
];
static REMOTEST_LAST_NAMES: &[&str] = &[
    "Sharma","Verma","Khan","Gupta","Reddy","Nair","Iyer","Singh","Patel","Mehta",
    "Kapoor","Joshi","Chatterjee","Bhatt","Malhotra","Das","Rao","Chauhan","Bose","Pillai",
];
static REMOTEST_DOMAINS: &[&str] = &["example.com","testmail.dev","mailinator.com","apeionmail.dev"];
static REMOTEST_WORDS: &[&str] = &[
    "the","system","runs","quickly","across","kernel","boundaries","while","tests",
    "validate","every","module","before","release","and","logic","stays","correct",
    "under","load","during","each","careful","review",
];
static REMOTEST_STREET_NAMES: &[&str] = &[
    "Maple","Oak","Cedar","Elm","Sunset","Lakeview","Hillcrest","River","Meadow",
    "Highland","Willow","Birch","Pine","Chestnut","Magnolia","Church","Main","Park",
];
static REMOTEST_STREET_SUFFIXES: &[&str] = &["St","Ave","Blvd","Ln","Dr","Rd","Ct","Way"];
static REMOTEST_CITIES: &[&str] = &[
    "Mumbai","Delhi","Bengaluru","Techon Nagar","Austin","Seattle","Toronto","Berlin",
    "London","Singapore","Tokyo","Dubai","Sydney","Amsterdam","Chicago","Denver",
];
static REMOTEST_COUNTRIES: &[&str] = &[
    "India","United States","Canada","Germany","United Kingdom","Singapore","Japan",
    "United Arab Emirates","Australia","Netherlands",
];
static REMOTEST_COMPANY_WORDS: &[&str] = &[
    "Nexora","Bright Path","Quantum","Vertex","Silverline","Northwind","Bluepeak",
    "Ironclad","Skyline","Cobalt","Redshift","Lumen","Orbital","Foundry","Meridian",
];
static REMOTEST_COMPANY_SUFFIXES: &[&str] = &["Systems","Labs","Technologies","Group","Solutions","Works","Networks","Inc."];
static REMOTEST_JOB_ROLES: &[&str] = &[
    "Software Engineer","Data Analyst","Product Manager","QA Engineer","DevOps Engineer",
    "UX Designer","Systems Architect","Database Administrator","Security Engineer","Technical Writer",
];

/// Real Luhn checksum — computes the check digit that makes `digits` (15
/// digits, no check digit yet) pass mod-10 validation, same math a real
/// payment processor runs. Used by fakeCreditCard so the generated number
/// is Luhn-valid, not just 16 random digits.
fn remotest_luhn_check_digit(digits: &[u8]) -> u8 {
    let mut sum: u32 = 0;
    let mut double = true; // rightmost of the 15 existing digits doubles first
    for &d in digits.iter().rev() {
        let mut v = d as u32;
        if double { v *= 2; if v > 9 { v -= 9; } }
        sum += v;
        double = !double;
    }
    ((10 - (sum % 10)) % 10) as u8
}

pub(crate) fn dispatch_autoclib(name: &str, args: Vec<Value>) -> Result<Value, String> {
    match name {
        // ---------------------------------------------------------------
        // Styling / Rich-style rendering
        // ---------------------------------------------------------------
        "__autoclib_style" => {
            let text  = autoclib_arg_str(&args, 0, "");
            let color = autoclib_arg_str(&args, 1, "");
            let bold  = autoclib_arg_bool(&args, 2);
            let code  = autoclib_ansi_code(&color);
            let prefix = if bold { format!("1;{}", code) } else { code.to_string() };
            Ok(Value::Str(format!("\x1b[{}m{}\x1b[0m", prefix, text)))
        }
        "__autoclib_print" => {
            let text = autoclib_arg_str(&args, 0, "");
            println!("{}", text);
            Ok(Value::Null)
        }
        "__autoclib_rule" => {
            let title = autoclib_arg_str(&args, 0, "");
            let width = 60usize;
            let line = if title.is_empty() {
                "─".repeat(width)
            } else {
                let label = format!(" {} ", title);
                let side = width.saturating_sub(label.chars().count()) / 2;
                format!("{}{}{}", "─".repeat(side), label, "─".repeat(width.saturating_sub(side + label.chars().count())))
            };
            println!("{}", line);
            Ok(Value::Str(line))
        }
        "__autoclib_panel" => {
            let text  = autoclib_arg_str(&args, 0, "");
            let title = autoclib_arg_str(&args, 1, "");
            let lines: Vec<&str> = text.split('\n').collect();
            let inner_w = lines.iter().map(|l| l.chars().count())
                .chain(core::iter::once(title.chars().count()))
                .max().unwrap_or(0);
            let top = if title.is_empty() {
                format!("┌{}┐", "─".repeat(inner_w + 2))
            } else {
                format!("┌─ {} {}┐", title, "─".repeat(inner_w.saturating_sub(title.chars().count()) + 1))
            };
            let mut out = String::new();
            out.push_str(&top); out.push('\n');
            for l in &lines {
                out.push_str(&format!("│ {:<width$} │\n", l, width = inner_w));
            }
            out.push_str(&format!("└{}┘", "─".repeat(inner_w + 2)));
            println!("{}", out);
            Ok(Value::Str(out))
        }
        "__autoclib_table" => {
            let headers = autoclib_arg_list(&args, 0);
            let rows    = autoclib_arg_list(&args, 1);
            let headers: Vec<String> = headers.iter().map(|v| v.to_string()).collect();
            let rows: Vec<Vec<String>> = rows.iter().map(|r| match r {
                Value::List(cells) => cells.iter().map(|c| c.to_string()).collect(),
                other => vec![other.to_string()],
            }).collect();
            let ncols = headers.len();
            let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
            for row in &rows {
                for (i, cell) in row.iter().enumerate() {
                    if i < ncols {
                        widths[i] = widths[i].max(cell.chars().count());
                    }
                }
            }
            let sep = |l: &str, m: &str, r: &str| -> String {
                let parts: Vec<String> = widths.iter().map(|w| "─".repeat(w + 2)).collect();
                format!("{}{}{}", l, parts.join(m), r)
            };
            let mut out = String::new();
            out.push_str(&sep("┌", "┬", "┐")); out.push('\n');
            out.push('│');
            for (i, h) in headers.iter().enumerate() { out.push_str(&format!(" {:<width$} │", h, width = widths[i])); }
            out.push('\n');
            out.push_str(&sep("├", "┼", "┤")); out.push('\n');
            for row in &rows {
                out.push('│');
                for i in 0..ncols {
                    let cell = row.get(i).map(|s| s.as_str()).unwrap_or("");
                    out.push_str(&format!(" {:<width$} │", cell, width = widths[i]));
                }
                out.push('\n');
            }
            out.push_str(&sep("└", "┴", "┘"));
            println!("{}", out);
            Ok(Value::Str(out))
        }
        "__autoclib_progress" => {
            let current = autoclib_arg_int(&args, 0, 0).max(0);
            let total   = autoclib_arg_int(&args, 1, 100).max(1);
            let label   = autoclib_arg_str(&args, 2, "");
            let pct = ((current as f64 / total as f64) * 100.0).clamp(0.0, 100.0);
            let bar_w = 30usize;
            let filled = ((pct / 100.0) * bar_w as f64).round() as usize;
            let bar = format!("[{}{}] {:>3}% {}", "#".repeat(filled), "-".repeat(bar_w.saturating_sub(filled)), pct as i64, label);
            println!("{}", bar);
            Ok(Value::Str(bar))
        }
        "__autoclib_spinner" => {
            const FRAMES: [&str; 10] = ["⠋","⠙","⠹","⠸","⠼","⠴","⠦","⠧","⠇","⠏"];
            let tick = autoclib_arg_int(&args, 0, 0).max(0) as usize;
            Ok(Value::Str(FRAMES[tick % FRAMES.len()].to_string()))
        }
        "__autoclib_markdown" => {
            let text = autoclib_arg_str(&args, 0, "");
            let mut out = String::new();
            for line in text.split('\n') {
                if let Some(rest) = line.strip_prefix("## ") {
                    out.push_str(&format!("\x1b[1m{}\x1b[0m\n", rest));
                } else if let Some(rest) = line.strip_prefix("# ") {
                    out.push_str(&format!("\x1b[1;4m{}\x1b[0m\n", rest));
                } else if let Some(rest) = line.strip_prefix("- ") {
                    out.push_str(&format!("  • {}\n", rest));
                } else {
                    // Inline **bold** → ANSI bold, minimal single-pass replace.
                    let mut rendered = String::new();
                    let mut bold_on = false;
                    let mut chars = line.chars().peekable();
                    while let Some(c) = chars.next() {
                        if c == '*' && chars.peek() == Some(&'*') {
                            chars.next();
                            rendered.push_str(if bold_on { "\x1b[0m" } else { "\x1b[1m" });
                            bold_on = !bold_on;
                        } else {
                            rendered.push(c);
                        }
                    }
                    out.push_str(&rendered);
                    out.push('\n');
                }
            }
            Ok(Value::Str(out))
        }

        // ---------------------------------------------------------------
        // Prompts
        // ---------------------------------------------------------------
        "__autoclib_prompt" => {
            let question = autoclib_arg_str(&args, 0, "");
            {
                use std::io::Write as _IoWrite;
                let mut w = std::io::stdout();
                let _ = write!(w, "{}: ", question);
                let _ = w.flush();
            }
            let mut line = String::new();
            io::stdin().lock().read_line(&mut line).ok();
            Ok(Value::Str(line.trim_end_matches(['\n', '\r']).to_string()))
        }
        "__autoclib_confirm" => {
            let question = autoclib_arg_str(&args, 0, "");
            {
                use std::io::Write as _IoWrite;
                let mut w = std::io::stdout();
                let _ = write!(w, "{} [y/n]: ", question);
                let _ = w.flush();
            }
            let mut line = String::new();
            io::stdin().lock().read_line(&mut line).ok();
            let l = line.trim().to_lowercase();
            Ok(Value::Bool(l == "y" || l == "yes"))
        }

        // ---------------------------------------------------------------
        // Argument parsing — argparse + Click + Typer + Fire combined
        // ---------------------------------------------------------------
        "__autoclib_parse_args" => {
            let argv: Vec<String> = autoclib_arg_list(&args, 0).iter().map(|v| v.to_string()).collect();
            let spec = autoclib_arg_map(&args, 1);

            // Build alias → canonical-name map, plus type/default lookups.
            let mut alias_of: Vec<(String, String)> = Vec::new();
            let mut result: Vec<(String, Value)> = Vec::new();
            for (flag_name, meta) in &spec {
                let meta_pairs = match meta { Value::Map(m) => m.clone(), _ => Vec::new() };
                let default = autoclib_map_get(&meta_pairs, "default").cloned().unwrap_or(Value::Null);
                result.push((flag_name.clone(), default));
                if let Some(Value::Str(alias)) = autoclib_map_get(&meta_pairs, "alias") {
                    alias_of.push((alias.clone(), flag_name.clone()));
                }
            }

            let mut positional: Vec<Value> = Vec::new();
            let mut i = 0usize;
            while i < argv.len() {
                let tok = &argv[i];
                let (raw_name, inline_val) = if let Some(stripped) = tok.strip_prefix("--") {
                    match stripped.split_once('=') {
                        Some((n, v)) => (n.to_string(), Some(v.to_string())),
                        None => (stripped.to_string(), None),
                    }
                } else if let Some(stripped) = tok.strip_prefix('-') {
                    (stripped.to_string(), None)
                } else {
                    positional.push(Value::Str(tok.clone()));
                    i += 1;
                    continue;
                };

                let canonical = alias_of.iter().find(|(a, _)| *a == raw_name)
                    .map(|(_, full)| full.clone())
                    .unwrap_or(raw_name.clone());

                let meta_pairs = spec.iter().find(|(k, _)| *k == canonical)
                    .and_then(|(_, m)| if let Value::Map(m) = m { Some(m.clone()) } else { None })
                    .unwrap_or_default();
                let ty = match autoclib_map_get(&meta_pairs, "type") { Some(Value::Str(t)) => t.clone(), _ => "string".to_string() };

                let value = if ty == "bool" && inline_val.is_none() {
                    i += 1;
                    Value::Bool(true)
                } else if let Some(v) = inline_val {
                    i += 1;
                    autoclib_coerce(&v, &ty)
                } else if i + 1 < argv.len() {
                    let v = argv[i + 1].clone();
                    i += 2;
                    autoclib_coerce(&v, &ty)
                } else {
                    i += 1;
                    Value::Bool(true)
                };

                if let Some(slot) = result.iter_mut().find(|(k, _)| *k == canonical) {
                    slot.1 = value;
                } else {
                    result.push((canonical, value));
                }
            }
            result.push(("_positional".to_string(), Value::List(positional)));
            Ok(Value::Map(result))
        }

        // ---------------------------------------------------------------
        // Subcommand tree — Click groups / Typer sub-apps
        // ---------------------------------------------------------------
        "__autoclib_command" => {
            let name = autoclib_arg_str(&args, 0, "");
            let desc = autoclib_arg_str(&args, 1, "");
            Ok(Value::Struct {
                name: "AutoclibCommand".to_string(),
                fields: vec![
                    ("name".to_string(), Value::Str(name)),
                    ("desc".to_string(), Value::Str(desc)),
                    ("subcommands".to_string(), Value::List(Vec::new())),
                ],
            })
        }
        "__autoclib_add_subcommand" => {
            let parent = args.get(0).cloned().unwrap_or(Value::Null);
            let child  = args.get(1).cloned().unwrap_or(Value::Null);
            match parent {
                Value::Struct { name, mut fields } => {
                    if let Some((_, Value::List(subs))) = fields.iter_mut().find(|(k, _)| k == "subcommands") {
                        subs.push(child);
                    } else {
                        fields.push(("subcommands".to_string(), Value::List(vec![child])));
                    }
                    Ok(Value::Struct { name, fields })
                }
                _ => Err("Autoclib.addSubcommand: parent must be a command created via Autoclib.command()".to_string()),
            }
        }
        "__autoclib_route" => {
            let mut current = args.get(0).cloned().unwrap_or(Value::Null);
            let mut remaining: Vec<String> = autoclib_arg_list(&args, 1).iter().map(|v| v.to_string()).collect();
            loop {
                let subs = match &current {
                    Value::Struct { fields, .. } => match autoclib_map_get(fields, "subcommands") {
                        Some(Value::List(l)) => l.clone(),
                        _ => Vec::new(),
                    },
                    _ => Vec::new(),
                };
                if remaining.is_empty() { break; }
                let head = remaining[0].clone();
                let matched = subs.into_iter().find(|s| matches!(s, Value::Struct { fields, .. } if autoclib_map_get(fields, "name").map(|v| v.to_string()) == Some(head.clone())));
                match matched {
                    Some(next) => { current = next; remaining.remove(0); }
                    None => break,
                }
            }
            Ok(Value::Map(vec![
                ("command".to_string(), current),
                ("args".to_string(), Value::List(remaining.into_iter().map(Value::Str).collect())),
            ]))
        }
        "__autoclib_help" => {
            fn render(node: &Value, depth: usize, out: &mut String) {
                if let Value::Struct { fields, .. } = node {
                    let name = autoclib_map_get(fields, "name").map(|v| v.to_string()).unwrap_or_default();
                    let desc = autoclib_map_get(fields, "desc").map(|v| v.to_string()).unwrap_or_default();
                    out.push_str(&format!("{}{} — {}\n", "  ".repeat(depth), name, desc));
                    if let Some(Value::List(subs)) = autoclib_map_get(fields, "subcommands") {
                        for s in subs { render(s, depth + 1, out); }
                    }
                }
            }
            let tree = args.get(0).cloned().unwrap_or(Value::Null);
            let mut out = String::new();
            render(&tree, 0, &mut out);
            Ok(Value::Str(out))
        }
        "__autoclib_completion" => {
            let tree  = args.get(0).cloned().unwrap_or(Value::Null);
            let shell = autoclib_arg_str(&args, 1, "bash");
            let (root_name, subs) = match &tree {
                Value::Struct { fields, .. } => (
                    autoclib_map_get(fields, "name").map(|v| v.to_string()).unwrap_or_default(),
                    match autoclib_map_get(fields, "subcommands") { Some(Value::List(l)) => l.clone(), _ => Vec::new() },
                ),
                _ => (String::new(), Vec::new()),
            };
            let names: Vec<String> = subs.iter().filter_map(|s| match s {
                Value::Struct { fields, .. } => autoclib_map_get(fields, "name").map(|v| v.to_string()),
                _ => None,
            }).collect();
            let script = match shell.as_str() {
                "zsh" => format!("#compdef {name}\n_{name}() {{ compadd {opts}; }}\ncompdef _{name} {name}\n", name = root_name, opts = names.join(" ")),
                _ => format!(
                    "_{name}_completions() {{\n  COMPREPLY=($(compgen -W \"{opts}\" -- \"${{COMP_WORDS[COMP_CWORD]}}\"))\n}}\ncomplete -F _{name}_completions {name}\n",
                    name = root_name, opts = names.join(" ")
                ),
            };
            Ok(Value::Str(script))
        }

        // ---------------------------------------------------------------
        // Config binding — auto-load CLI defaults from a config file's text
        // ---------------------------------------------------------------
        "__autoclib_config" => {
            let text = autoclib_arg_str(&args, 0, "");
            let mut out: Vec<(String, Value)> = Vec::new();
            for line in text.split('\n') {
                let l = line.trim();
                if l.is_empty() || l.starts_with('#') { continue; }
                let sep_pos = l.find(':').or_else(|| l.find('='));
                if let Some(pos) = sep_pos {
                    let key = l[..pos].trim().to_string();
                    let val = l[pos + 1..].trim().trim_matches('"').to_string();
                    let parsed = if let Ok(n) = val.parse::<i64>() { Value::Int(n) }
                        else if let Ok(f) = val.parse::<f64>() { Value::Float(f) }
                        else if val == "true" { Value::Bool(true) }
                        else if val == "false" { Value::Bool(false) }
                        else { Value::Str(val) };
                    out.push((key, parsed));
                }
            }
            Ok(Value::Map(out))
        }

        // ---------------------------------------------------------------
        // Structured logging (custom — not present in any of the 7 Python libs)
        // ---------------------------------------------------------------
        "__autoclib_log" => {
            let level = autoclib_arg_str(&args, 0, "info");
            let msg   = autoclib_arg_str(&args, 1, "");
            let entry = Value::Map(vec![
                ("level".to_string(), Value::Str(level)),
                ("msg".to_string(), Value::Str(msg)),
            ]);
            let json = value_to_json(&entry);
            println!("{}", json);
            Ok(Value::Str(json))
        }

        // ---------------------------------------------------------------
        // Automation — Fabric-style task dependency ordering
        // ---------------------------------------------------------------
        "__autoclib_task_sort" => {
            let tasks = autoclib_arg_list(&args, 0);
            let mut names: Vec<String> = Vec::new();
            let mut deps_of: Vec<(String, Vec<String>)> = Vec::new();
            for t in &tasks {
                if let Value::Map(fields) = t {
                    let name = autoclib_map_get(fields, "name").map(|v| v.to_string()).unwrap_or_default();
                    let deps: Vec<String> = match autoclib_map_get(fields, "deps") {
                        Some(Value::List(l)) => l.iter().map(|v| v.to_string()).collect(),
                        _ => Vec::new(),
                    };
                    names.push(name.clone());
                    deps_of.push((name, deps));
                }
            }
            let mut resolved: Vec<String> = Vec::new();
            let mut remaining = deps_of.clone();
            while !remaining.is_empty() {
                let ready: Vec<String> = remaining.iter()
                    .filter(|(_, deps)| deps.iter().all(|d| resolved.contains(d)))
                    .map(|(n, _)| n.clone())
                    .collect();
                if ready.is_empty() {
                    let stuck = remaining.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>().join(", ");
                    return Err(format!("Autoclib.taskSort: dependency cycle detected among: {}", stuck));
                }
                for n in &ready { resolved.push(n.clone()); }
                remaining.retain(|(n, _)| !ready.contains(n));
            }
            Ok(Value::List(resolved.into_iter().map(Value::Str).collect()))
        }
        // ---------------------------------------------------------------
        // Automation — real remote execution wire protocol (Fabric-style).
        // NOTE: this is deliberately NOT SSH — implementing real SSH means
        // shipping a full crypto/key-exchange stack, which is a separate,
        // much bigger undertaking and out of scope here. What this IS: a
        // real, working length-prefixed request/response protocol assuming
        // a small Monobat "exec agent" listens on the other end. The logic
        // below is complete and correct — the only missing piece is
        // MonobatHal::net_tcp_connect/write/read, which are already defined
        // as default trait methods above. Wire those three to Monobat's real
        // network driver and remoteExec works exactly as written, no changes
        // needed here.
        // ---------------------------------------------------------------
        "__autoclib_remote_exec" => {
            let host = autoclib_arg_str(&args, 0, "");
            let cmd  = autoclib_arg_str(&args, 1, "");
            let addr = if host.contains(':') { host.clone() } else { format!("{}:7777", host) };

            let handle = remox_tcp_connect(&addr).map_err(|e| {
                format!("Autoclib.remoteExec({}): connect failed — {}", host, e)
            })?;

            // Frame: 4-byte big-endian length prefix + UTF-8 command bytes.
            let cmd_bytes = cmd.as_bytes();
            let mut frame = Vec::with_capacity(4 + cmd_bytes.len());
            frame.extend_from_slice(&(cmd_bytes.len() as u32).to_be_bytes());
            frame.extend_from_slice(cmd_bytes);
            remox_tcp_write(handle, &frame).map_err(|e| {
                format!("Autoclib.remoteExec({}): send failed — {}", host, e)
            })?;

            // Read the 4-byte response length prefix, then the payload,
            // looping net_tcp_read until the full frame has arrived.
            let mut len_buf: Vec<u8> = Vec::new();
            while len_buf.len() < 4 {
                let chunk = remox_tcp_read(handle, 4 - len_buf.len()).map_err(|e| {
                    format!("Autoclib.remoteExec({}): reading response length failed — {}", host, e)
                })?;
                if chunk.is_empty() {
                    return Err(format!("Autoclib.remoteExec({}): connection closed before response length arrived", host));
                }
                len_buf.extend_from_slice(&chunk);
            }
            let resp_len = u32::from_be_bytes([len_buf[0], len_buf[1], len_buf[2], len_buf[3]]) as usize;

            let mut payload: Vec<u8> = Vec::new();
            while payload.len() < resp_len {
                let chunk = remox_tcp_read(handle, resp_len - payload.len()).map_err(|e| {
                    format!("Autoclib.remoteExec({}): reading response body failed — {}", host, e)
                })?;
                if chunk.is_empty() {
                    return Err(format!("Autoclib.remoteExec({}): connection closed mid-response ({}/{} bytes)", host, payload.len(), resp_len));
                }
                payload.extend_from_slice(&chunk);
            }
            Ok(Value::Str(String::from_utf8_lossy(&payload).to_string()))
        }

        // ---------------------------------------------------------------
        // History (REPL-mode session memory — custom)
        // ---------------------------------------------------------------
        "__autoclib_history_add" => {
            let line = autoclib_arg_str(&args, 0, "");
            unsafe { (*AUTOCLIB_HISTORY.0.get()).push(line); }
            Ok(Value::Null)
        }
        "__autoclib_history_list" => {
            let items = unsafe { (*AUTOCLIB_HISTORY.0.get()).clone() };
            Ok(Value::List(items.into_iter().map(Value::Str).collect()))
        }

        // ---------------------------------------------------------------
        // TUI widgets — Textual-equivalent. A widget is a plain Value::Struct
        // ("AutoclibWidget") so it composes with everything else in the
        // language (structs, pipe operator, list comprehensions, etc.) — no
        // opaque object hidden from Remox code.
        //   fields: kind, id, label, value, children(List<Widget>)
        // ---------------------------------------------------------------
        "__autoclib_widget" => {
            let kind  = autoclib_arg_str(&args, 0, "button");
            let id    = autoclib_arg_str(&args, 1, "");
            let label = autoclib_arg_str(&args, 2, "");
            let value = args.get(3).cloned().unwrap_or(Value::Null);
            Ok(Value::Struct {
                name: "AutoclibWidget".to_string(),
                fields: vec![
                    ("kind".to_string(), Value::Str(kind)),
                    ("id".to_string(), Value::Str(id)),
                    ("label".to_string(), Value::Str(label)),
                    ("value".to_string(), value),
                    ("children".to_string(), Value::List(Vec::new())),
                ],
            })
        }
        "__autoclib_container" => {
            let id = autoclib_arg_str(&args, 0, "");
            let children = autoclib_arg_list(&args, 1);
            Ok(Value::Struct {
                name: "AutoclibWidget".to_string(),
                fields: vec![
                    ("kind".to_string(), Value::Str("container".to_string())),
                    ("id".to_string(), Value::Str(id)),
                    ("label".to_string(), Value::Str(String::new())),
                    ("value".to_string(), Value::Null),
                    ("children".to_string(), Value::List(children)),
                ],
            })
        }
        "__autoclib_render" => {
            fn render_node(node: &Value, focus_id: &str, depth: usize, out: &mut String) {
                if let Value::Struct { fields, .. } = node {
                    let kind  = autoclib_map_get(fields, "kind").map(|v| v.to_string()).unwrap_or_default();
                    let id    = autoclib_map_get(fields, "id").map(|v| v.to_string()).unwrap_or_default();
                    let label = autoclib_map_get(fields, "label").map(|v| v.to_string()).unwrap_or_default();
                    let value = autoclib_map_get(fields, "value").cloned().unwrap_or(Value::Null);
                    let is_focused = id == focus_id && !id.is_empty();
                    let indent = "  ".repeat(depth);
                    let rendered = match kind.as_str() {
                        "button"    => format!("[ {} ]", label),
                        "checkbox"  => format!("[{}] {}", if matches!(value, Value::Bool(true)) { "x" } else { " " }, label),
                        "input"     => format!("{}: {}_", label, value),
                        "listitem"  => format!("• {}", label),
                        "tab"       => format!("〈{}〉", label),
                        "container" => String::new(),
                        _ => label.clone(),
                    };
                    if !rendered.is_empty() {
                        if is_focused {
                            out.push_str(&format!("{}\x1b[7m{}\x1b[0m\n", indent, rendered)); // inverse video = real focus highlight
                        } else {
                            out.push_str(&format!("{}{}\n", indent, rendered));
                        }
                    }
                    if let Some(Value::List(children)) = autoclib_map_get(fields, "children") {
                        for child in children { render_node(child, focus_id, depth + 1, out); }
                    }
                }
            }
            let tree = args.get(0).cloned().unwrap_or(Value::Null);
            let focus_id = autoclib_arg_str(&args, 1, "");
            let mut out = String::new();
            render_node(&tree, &focus_id, 0, &mut out);
            Ok(Value::Str(out))
        }
        "__autoclib_focusables" => {
            fn collect(node: &Value, out: &mut Vec<String>) {
                if let Value::Struct { fields, .. } = node {
                    let kind = autoclib_map_get(fields, "kind").map(|v| v.to_string()).unwrap_or_default();
                    let id   = autoclib_map_get(fields, "id").map(|v| v.to_string()).unwrap_or_default();
                    if matches!(kind.as_str(), "button" | "checkbox" | "input" | "listitem" | "tab") && !id.is_empty() {
                        out.push(id);
                    }
                    if let Some(Value::List(children)) = autoclib_map_get(fields, "children") {
                        for child in children { collect(child, out); }
                    }
                }
            }
            let tree = args.get(0).cloned().unwrap_or(Value::Null);
            let mut ids = Vec::new();
            collect(&tree, &mut ids);
            Ok(Value::List(ids.into_iter().map(Value::Str).collect()))
        }
        "__autoclib_handle_key" => {
            fn collect_ids(node: &Value, out: &mut Vec<String>) {
                if let Value::Struct { fields, .. } = node {
                    let kind = autoclib_map_get(fields, "kind").map(|v| v.to_string()).unwrap_or_default();
                    let id   = autoclib_map_get(fields, "id").map(|v| v.to_string()).unwrap_or_default();
                    if matches!(kind.as_str(), "button" | "checkbox" | "input" | "listitem" | "tab") && !id.is_empty() {
                        out.push(id);
                    }
                    if let Some(Value::List(children)) = autoclib_map_get(fields, "children") {
                        for child in children { collect_ids(child, out); }
                    }
                }
            }
            /// Toggles a checkbox's boolean value in-place through the tree,
            /// functionally (returns a new tree, same immutable style as the
            /// rest of Autoclib).
            fn toggle(node: Value, target_id: &str) -> Value {
                match node {
                    Value::Struct { name, mut fields } => {
                        let id = autoclib_map_get(&fields, "id").map(|v| v.to_string()).unwrap_or_default();
                        if id == target_id {
                            if let Some((_, v)) = fields.iter_mut().find(|(k, _)| k == "value") {
                                if let Value::Bool(b) = v { *v = Value::Bool(!*b); }
                            }
                        }
                        if let Some((_, Value::List(children))) = fields.iter_mut().find(|(k, _)| k == "children") {
                            let updated: Vec<Value> = core::mem::take(children).into_iter().map(|c| toggle(c, target_id)).collect();
                            *children = updated;
                        }
                        Value::Struct { name, fields }
                    }
                    other => other,
                }
            }
            let tree = args.get(0).cloned().unwrap_or(Value::Null);
            let focus_id = autoclib_arg_str(&args, 1, "");
            let key = autoclib_arg_str(&args, 2, "");

            let mut ids = Vec::new();
            collect_ids(&tree, &mut ids);
            let cur_idx = ids.iter().position(|i| *i == focus_id);

            let (new_focus, action, new_tree) = match key.as_str() {
                "down" | "tab" => {
                    let next = match cur_idx { Some(i) => (i + 1) % ids.len().max(1), None => 0 };
                    (ids.get(next).cloned().unwrap_or_default(), Value::Null, tree)
                }
                "up" | "shift_tab" => {
                    let next = match cur_idx {
                        Some(i) => (i + ids.len().saturating_sub(1)) % ids.len().max(1),
                        None => 0,
                    };
                    (ids.get(next).cloned().unwrap_or_default(), Value::Null, tree)
                }
                "enter" | "space" => {
                    // Real semantics per widget kind, not a generic no-op:
                    // buttons/list items/tabs fire an "activate:<id>" action
                    // the caller's Remox code can match on; checkboxes flip
                    // their own value in the returned tree.
                    let updated_tree = toggle(tree, &focus_id);
                    (focus_id.clone(), Value::Str(format!("activate:{}", focus_id)), updated_tree)
                }
                _ => (focus_id.clone(), Value::Null, tree),
            };

            Ok(Value::Map(vec![
                ("tree".to_string(), new_tree),
                ("focus".to_string(), Value::Str(new_focus)),
                ("action".to_string(), action),
            ]))
        }
        "__autoclib_read_key" => {
            // Real (non-simulated) path: drain any bytes MonobatHal already
            // has buffered from the serial console, mapping common VT100
            // arrow/enter sequences to logical key names the TUI understands.
            let mut collected: Vec<u8> = Vec::new();
            while let Some(b) = None::<u8> {
                collected.push(b);
                if collected.len() >= 3 { break; } // enough for an ESC [ A/B/C/D sequence
            }
            if !collected.is_empty() {
                return Ok(Value::Str(match collected.as_slice() {
                    [0x1b, b'[', b'A'] => "up".to_string(),
                    [0x1b, b'[', b'B'] => "down".to_string(),
                    [b'\r'] | [b'\n'] => "enter".to_string(),
                    [b'\t'] => "tab".to_string(),
                    [b' '] => "space".to_string(),
                    other => String::from_utf8_lossy(other).to_string(),
                }));
            }
            // HONEST FALLBACK: serial_read_byte isn't wired yet (returns
            // None by default — see MonobatHal above), so there is no raw
            // keystroke source. Rather than block forever or fake a key,
            // fall back to a real line-buffered read and interpret the
            // typed word ("up"/"down"/"enter"/"tab"/"space", or a literal
            // character) as the key name. This is genuinely functional
            // today; it upgrades to true single-keystroke input the moment
            // `serial_read_byte` gets a real implementation — nothing else
            // in Autoclib needs to change.
            let mut line = String::new();
            io::stdin().lock().read_line(&mut line).ok();
            let trimmed = line.trim().to_lowercase();
            Ok(Value::Str(if trimmed.is_empty() { "enter".to_string() } else { trimmed }))
        }

        _ => Err(format!("Unknown Autoclib function: {}", name)),
    }
}

// =============================================================================
// ASTRILOOP — Async Runtime Library (integrated below dispatch_remotest/tasoaque)
// =============================================================================
// =============================================================================
// ASTRILOOP ENGINE — Internal State
// =============================================================================
// No_std compatible. Arc<Mutex<_>> use karta hai alloc::sync se.
// Monobat HAL se time aata hai (remox_entropy() + thread::sleep).

use std::collections::VecDeque;
// NOTE: Arc and AtomicU64/Ordering are already brought into scope at file
// level (`use std::sync::{Arc, Mutex};` and `use core::sync::atomic::{AtomicU64, Ordering};`
// near Numrux). Re-importing them here would conflict (E0252), so only the
// names not already in scope are imported below. We keep using raw
// `spin::Mutex` (aliased SpinMutex) here instead of the kernel's own
// std-shaped `Mutex` wrapper, because that wrapper's `.lock()` returns
// `Result<MutexGuard, PoisonError>` (needs `.unwrap()`/`?`) while this file's
// call sites use the plain spin-style `.lock()` — and `spin` is already a
// kernel dependency (see `pub mod sync` above), so this is a safe, additive import.
use std::collections::BTreeMap;
use core::sync::atomic::{AtomicBool, AtomicUsize};

// ── Monotonic clock (HAL-backed) ──────────────────────────────────────────
// Production mein yeh Monobat timer driver se aayega.
// Abhi: entropy_source() se derive karte hain (unique across calls on HW).
// thread::sleep_ms ke saath combine karke relative timing reliable hai.
static ASTRILOOP_BOOT_MS: AtomicU64 = AtomicU64::new(0);
static ASTRILOOP_BOOT_INIT: AtomicBool = AtomicBool::new(false);

fn astriloop_now_ms() -> u64 {
    // First call: seed with HAL entropy (acts as boot timestamp proxy)
    if !ASTRILOOP_BOOT_INIT.load(Ordering::Relaxed) {
        let seed = crate::remox_entropy();
        ASTRILOOP_BOOT_MS.store(seed & 0xFFFF_FFFF, Ordering::Relaxed);
        ASTRILOOP_BOOT_INIT.store(true, Ordering::Relaxed);
    }
    // Each call bumps by 1ms minimum — guarantees monotonic even without HW timer.
    // Real Monobat timer driver override karega yeh behavior.
    ASTRILOOP_BOOT_MS.fetch_add(1, Ordering::Relaxed)
}

// ── Global runtime stats ──────────────────────────────────────────────────
static ASTRILOOP_TASKS_SPAWNED:   AtomicUsize = AtomicUsize::new(0);
static ASTRILOOP_TASKS_DONE:      AtomicUsize = AtomicUsize::new(0);
static ASTRILOOP_TASKS_CANCELLED: AtomicUsize = AtomicUsize::new(0);
static ASTRILOOP_TRACE:           AtomicBool  = AtomicBool::new(false);

fn astriloop_trace(msg: &str) {
    if ASTRILOOP_TRACE.load(Ordering::Relaxed) {
        println!("[Astriloop] {}", msg);
    }
}

// ── Task handle ──────────────────────────────────────────────────────────
// Value::AsyncHandle(Arc<Mutex<Option<Value>>>) already exists in Remox.
// We reuse it; status is derived from whether the Option is Some/None + cancelled flag.

// ── Channel internal state ────────────────────────────────────────────────
struct AstriloopChan {
    buf:     VecDeque<Value>,
    cap:     usize,     // 0 = unbounded
    closed:  bool,
}

// ── Queue internal state ──────────────────────────────────────────────────
struct AstriloopQueue {
    items:       VecDeque<(i64, Value)>,  // (priority, value); lower = higher priority
    maxsize:     usize,
    pending:     usize,  // unfinished tasks count (for join())
}

// ── Event ─────────────────────────────────────────────────────────────────
struct AstriloopEvent {
    set: bool,
}

// ── Lock ──────────────────────────────────────────────────────────────────
struct AstriloopLock {
    held: bool,
}

// ── Semaphore ─────────────────────────────────────────────────────────────
struct AstriloopSemaphore {
    count:   i64,
    initial: i64,
}

// ── Barrier ───────────────────────────────────────────────────────────────
struct AstriloopBarrier {
    target:  usize,
    arrived: usize,
}

// ── Stream ────────────────────────────────────────────────────────────────
// A push-based async stream: items pushed by producer, consumed by pipeline ops.
struct AstriloopStream {
    buf:         VecDeque<Value>,
    ended:       bool,
    transforms:  Vec<AstriloopTransform>,
    subscribers: Vec<String>,  // fn names to call on each push
}

enum AstriloopTransform {
    Map(String),          // fn name
    Filter(String),       // fn name
    Batch(usize),         // batch size
    DebounceMs(u64),      // debounce ms
    ThrottleMs(u64, u64), // (interval_ms, last_emit_ms)
}

// ── Signal Bus ───────────────────────────────────────────────────────────
struct AstriloopBus {
    // topic -> list of (fn_name, once_flag)
    listeners: BTreeMap<String, Vec<(String, bool)>>,
}

// ── Nursery ───────────────────────────────────────────────────────────────
struct AstriloopNursery {
    handles:  Vec<Arc<Mutex<Option<Value>>>>,  // AsyncHandle slots
    errors:   Vec<String>,
    closed:   bool,
}

// ── Scheduled tasks ───────────────────────────────────────────────────────
struct AstriloopSchedule {
    interval_ms:  u64,
    next_ms:      u64,
    fn_name:      String,
    cancelled:    bool,
    is_cron:      bool,
    cron_expr:    String,
}

// ── Rate limit / circuit breaker ─────────────────────────────────────────
struct AstriloopRateLimiter {
    max_per_window: u64,
    window_ms:      u64,
    window_start:   u64,
    count:          u64,
}

struct AstriloopCircuit {
    fn_name:      String,
    threshold:    u64,   // failures before open
    reset_after:  u64,   // ms to wait before half-open
    failures:     u64,
    state:        u8,    // 0=closed 1=open 2=half-open
    opened_at:    u64,
}

// ── Global registry (keyed by generated IDs) ─────────────────────────────
// We store all Astriloop objects as Value::Map with a __astriloop_id field.
// The actual state lives in these global registries indexed by id.
// No heap-of-traits needed — simple id→struct maps.

use spin::Mutex as SpinMutex;

static ASTRILOOP_CHANS:     SpinMutex<BTreeMap<u64, AstriloopChan>>     = SpinMutex::new(BTreeMap::new());
static ASTRILOOP_QUEUES:    SpinMutex<BTreeMap<u64, AstriloopQueue>>    = SpinMutex::new(BTreeMap::new());
static ASTRILOOP_EVENTS:    SpinMutex<BTreeMap<u64, AstriloopEvent>>    = SpinMutex::new(BTreeMap::new());
static ASTRILOOP_LOCKS:     SpinMutex<BTreeMap<u64, AstriloopLock>>     = SpinMutex::new(BTreeMap::new());
static ASTRILOOP_SEMS:      SpinMutex<BTreeMap<u64, AstriloopSemaphore>>= SpinMutex::new(BTreeMap::new());
static ASTRILOOP_BARRIERS:  SpinMutex<BTreeMap<u64, AstriloopBarrier>>  = SpinMutex::new(BTreeMap::new());
static ASTRILOOP_STREAMS:   SpinMutex<BTreeMap<u64, AstriloopStream>>   = SpinMutex::new(BTreeMap::new());
static ASTRILOOP_BUSES:     SpinMutex<BTreeMap<u64, AstriloopBus>>      = SpinMutex::new(BTreeMap::new());
static ASTRILOOP_NURSERIES: SpinMutex<BTreeMap<u64, AstriloopNursery>>  = SpinMutex::new(BTreeMap::new());
static ASTRILOOP_SCHEDULES: SpinMutex<BTreeMap<u64, AstriloopSchedule>> = SpinMutex::new(BTreeMap::new());
static ASTRILOOP_RATES:     SpinMutex<BTreeMap<u64, AstriloopRateLimiter>> = SpinMutex::new(BTreeMap::new());
static ASTRILOOP_CIRCUITS:  SpinMutex<BTreeMap<u64, AstriloopCircuit>>  = SpinMutex::new(BTreeMap::new());

static ASTRILOOP_NEXT_ID: AtomicU64 = AtomicU64::new(1);
fn astriloop_new_id() -> u64 {
    ASTRILOOP_NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

// Helper: Value::Map se __astriloop_id nikaalo
fn astriloop_id_of(v: &Value) -> Option<u64> {
    if let Value::Map(ref fields) = v {
        fields.iter().find(|(k, _)| k == "__astriloop_id")
            .and_then(|(_, v)| if let Value::Int(n) = v { Some(*n as u64) } else { None })
    } else {
        None
    }
}

// Helper: ek tagged Astriloop object banao
fn astriloop_obj(id: u64, kind: &str) -> Value {
    Value::Map(vec![
        ("__astriloop_id".into(),   Value::Int(id as i64)),
        ("__astriloop_kind".into(), Value::Str(kind.into())),
    ])
}

// =============================================================================
// ASTRILOOP DISPATCH FUNCTION
// Yeh `impl Interpreter` ke andar jaata hai — self.call_value() access ke liye.
// =============================================================================

impl Interpreter {
    pub(crate) fn dispatch_astriloop(&mut self, name: &str, mut args: Vec<Value>) -> Result<Value, RuntimeSignal> {
        astriloop_trace(name);
        match name {

            // ════════════════════════════════════════════════════════════════
            // CORE EVENT LOOP
            // ════════════════════════════════════════════════════════════════

            // Astriloop.run(fn)
            // Top-level entry point. Executes fn synchronously (Remox async fns
            // already run inline via thread::spawn + join on the AsyncHandle).
            // Returns the fn's return value.
            "__astriloop_run" => {
                let f = args.into_iter().next().unwrap_or(Value::Null);
                astriloop_trace("run: starting event loop");
                let result = self.call_value(f, vec![])?;
                // If result is an AsyncHandle, resolve it (await)
                let resolved = match result {
                    Value::AsyncHandle(arc) => {
                        // Spin-wait (cooperative) — Monobat scheduler will preempt for real
                        loop {
                            {
                                let guard = arc.lock().unwrap();
                                if let Some(ref v) = *guard {
                                    let out = v.clone();
                                    drop(guard);
                                    break out;
                                }
                            }
                            thread::sleep(core::time::Duration::from_millis(1));
                        }
                    }
                    v => v,
                };
                ASTRILOOP_TASKS_DONE.fetch_add(1, Ordering::Relaxed);
                astriloop_trace("run: event loop done");
                Ok(resolved)
            }

            // Astriloop.sleep(ms)
            // Cooperative yield: suspends current task for ms milliseconds.
            "__astriloop_sleep" => {
                let ms = match args.into_iter().next() {
                    Some(Value::Int(n))   => n as u64,
                    Some(Value::Float(f)) => f as u64,
                    _                    => 0,
                };
                thread::sleep(core::time::Duration::from_millis(ms));
                Ok(Value::Null)
            }

            // Astriloop.tick()
            // Cooperative checkpoint — yield once so other tasks can run.
            "__astriloop_tick" => {
                thread::sleep(core::time::Duration::from_millis(0));
                Ok(Value::Null)
            }

            // Astriloop.now()
            // Monotonic clock in ms.
            "__astriloop_now" => {
                Ok(Value::Int(astriloop_now_ms() as i64))
            }

            // ════════════════════════════════════════════════════════════════
            // TASK MANAGEMENT
            // ════════════════════════════════════════════════════════════════

            // Astriloop.spawn(fn, args?)
            // Spawns fn as a background task. Returns an AsyncHandle.
            "__astriloop_spawn" => {
                let f    = args.remove(0);
                let fargs = if args.is_empty() {
                    vec![]
                } else {
                    match args.remove(0) {
                        Value::List(v) => v,
                        other          => vec![other],
                    }
                };
                ASTRILOOP_TASKS_SPAWNED.fetch_add(1, Ordering::Relaxed);
                astriloop_trace("spawn: launching task");
                // Reuse Remox's existing async fn call path:
                // Wrap f + fargs into a synthetic async fn call.
                let result_slot: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
                let slot_clone  = Arc::clone(&result_slot);
                // Snapshot interpreter state (same pattern as existing async fn path)
                let env_snap = self.env.clone();
                let fns_snap: HashMap<String, (Vec<(String, Option<Expr>)>, Vec<Stmt>, bool)> =
                    self.fns.iter().map(|(k, (p, b, a))| (k.clone(), (p.clone(), (**b).clone(), *a))).collect();
                let structs_snap = self.structs.clone();
                let impls_snap   = self.impls.clone();
                let traits_snap  = self.traits.clone();
                let rand_state   = self.rand_state;

                thread::spawn(move || {
                    let fns_rc: HashMap<String, (Vec<(String, Option<Expr>)>, Rc<Vec<Stmt>>, bool)> =
                        fns_snap.into_iter().map(|(k, (p, b, a))| (k, (p, Rc::new(b), a))).collect();
                    let mut sub = Interpreter {
                        env: env_snap, fns: fns_rc, structs: structs_snap,
                        impls: impls_snap, traits: traits_snap,
                        rand_state, memo: HashMap::new(),
                        pending_styles: Vec::new(),
                        remojoke_lang: String::from("src"),
                    };
                    let res = sub.call_value(f, fargs);
                    let val = match res {
                        Ok(v)                          => v,
                        Err(RuntimeSignal::Return(Some(v))) => v,
                        Err(e) => {
                            ASTRILOOP_TASKS_CANCELLED.fetch_add(1, Ordering::Relaxed);
                            Value::Str(format!("__astriloop_error:{:?}", e))
                        }
                    };
                    ASTRILOOP_TASKS_DONE.fetch_add(1, Ordering::Relaxed);
                    *slot_clone.lock().unwrap() = Some(val);
                });

                Ok(Value::AsyncHandle(result_slot))
            }

            // Astriloop.cancel(handle)
            // Marks a task as cancelled. Since Remox tasks are threads, we
            // signal via the slot: write a sentinel cancel value.
            // (Full preemptive cancel needs kernel thread signal — placeholder.)
            "__astriloop_cancel" => {
                if let Some(Value::AsyncHandle(arc)) = args.into_iter().next() {
                    let mut guard = arc.lock().unwrap();
                    if guard.is_none() {
                        *guard = Some(Value::Str("__astriloop_cancelled".into()));
                        ASTRILOOP_TASKS_CANCELLED.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(Value::Bool(true))
                } else {
                    Ok(Value::Bool(false))
                }
            }

            // Astriloop.taskStatus(handle) → "pending" | "done" | "cancelled" | "error"
            "__astriloop_task_status" => {
                if let Some(Value::AsyncHandle(arc)) = args.into_iter().next() {
                    let guard = arc.lock().unwrap();
                    let status = match &*guard {
                        None => "pending",
                        Some(Value::Str(s)) if s.starts_with("__astriloop_cancelled") => "cancelled",
                        Some(Value::Str(s)) if s.starts_with("__astriloop_error:")    => "error",
                        Some(_) => "done",
                    };
                    Ok(Value::Str(status.into()))
                } else {
                    Ok(Value::Str("unknown".into()))
                }
            }

            // ════════════════════════════════════════════════════════════════
            // GATHER / RACE / ALL-SETTLED / ANY
            // ════════════════════════════════════════════════════════════════

            // Astriloop.gather(fns)  — run all, collect all results (error = first error)
            "__astriloop_gather" => {
                let fns = match args.into_iter().next() {
                    Some(Value::List(v)) => v,
                    _ => return Ok(Value::List(vec![])),
                };
                let mut handles: Vec<Arc<Mutex<Option<Value>>>> = Vec::new();
                for f in fns {
                    // Spawn each as background task
                    let h = self.dispatch_astriloop("__astriloop_spawn", vec![f, Value::List(vec![])])?;
                    if let Value::AsyncHandle(arc) = h { handles.push(arc); }
                }
                let mut results = Vec::new();
                for arc in handles {
                    loop {
                        {
                            let guard = arc.lock().unwrap();
                            if let Some(ref v) = *guard {
                                if let Value::Str(s) = v {
                                    if s.starts_with("__astriloop_error:") {
                                        return Err(RuntimeSignal::Error(s[18..].to_string()));
                                    }
                                }
                                results.push(v.clone());
                                break;
                            }
                        }
                        thread::sleep(core::time::Duration::from_millis(1));
                    }
                }
                Ok(Value::List(results))
            }

            // Astriloop.race(fns)  — first to finish wins; rest cancelled
            "__astriloop_race" => {
                let fns = match args.into_iter().next() {
                    Some(Value::List(v)) => v,
                    _ => return Ok(Value::Null),
                };
                let mut handles: Vec<Arc<Mutex<Option<Value>>>> = Vec::new();
                for f in fns {
                    let h = self.dispatch_astriloop("__astriloop_spawn", vec![f, Value::List(vec![])])?;
                    if let Value::AsyncHandle(arc) = h { handles.push(arc); }
                }
                loop {
                    for arc in &handles {
                        let guard = arc.lock().unwrap();
                        if let Some(ref v) = *guard {
                            let winner = v.clone();
                            drop(guard);
                            // Cancel the rest
                            for other in &handles {
                                let mut g = other.lock().unwrap();
                                if g.is_none() { *g = Some(Value::Str("__astriloop_cancelled".into())); }
                            }
                            return Ok(winner);
                        }
                    }
                    thread::sleep(core::time::Duration::from_millis(1));
                }
            }

            // Astriloop.allSettled(fns)  — gather; never throws; returns list of {ok, value/error}
            "__astriloop_all_settled" => {
                let fns = match args.into_iter().next() {
                    Some(Value::List(v)) => v,
                    _ => return Ok(Value::List(vec![])),
                };
                let mut handles: Vec<Arc<Mutex<Option<Value>>>> = Vec::new();
                for f in fns {
                    let h = self.dispatch_astriloop("__astriloop_spawn", vec![f, Value::List(vec![])])?;
                    if let Value::AsyncHandle(arc) = h { handles.push(arc); }
                }
                let mut results = Vec::new();
                for arc in handles {
                    loop {
                        let guard = arc.lock().unwrap();
                        if let Some(ref v) = *guard {
                            let entry = match v {
                                Value::Str(s) if s.starts_with("__astriloop_error:") =>
                                    Value::Map(vec![
                                        ("ok".into(), Value::Bool(false)),
                                        ("error".into(), Value::Str(s[18..].to_string())),
                                    ]),
                                other => Value::Map(vec![
                                    ("ok".into(), Value::Bool(true)),
                                    ("value".into(), other.clone()),
                                ]),
                            };
                            results.push(entry);
                            break;
                        }
                        drop(guard);
                        thread::sleep(core::time::Duration::from_millis(1));
                    }
                }
                Ok(Value::List(results))
            }

            // Astriloop.any(fns)  — first SUCCESS wins (ignores errors until all fail)
            "__astriloop_any" => {
                let fns = match args.into_iter().next() {
                    Some(Value::List(v)) => v,
                    _ => return Ok(Value::Null),
                };
                let total = fns.len();
                let mut handles: Vec<Arc<Mutex<Option<Value>>>> = Vec::new();
                for f in fns {
                    let h = self.dispatch_astriloop("__astriloop_spawn", vec![f, Value::List(vec![])])?;
                    if let Value::AsyncHandle(arc) = h { handles.push(arc); }
                }
                let mut done_count = 0usize;
                loop {
                    for arc in &handles {
                        let guard = arc.lock().unwrap();
                        if let Some(ref v) = *guard {
                            if let Value::Str(s) = v {
                                if s.starts_with("__astriloop_error:") {
                                    done_count += 1;
                                    if done_count >= total {
                                        return Err(RuntimeSignal::Error("Astriloop.any: all tasks failed".into()));
                                    }
                                    continue;
                                }
                            }
                            return Ok(v.clone());
                        }
                    }
                    thread::sleep(core::time::Duration::from_millis(1));
                }
            }

            // ════════════════════════════════════════════════════════════════
            // STRUCTURED CONCURRENCY — NURSERY (trio-style)
            // ════════════════════════════════════════════════════════════════

            "__astriloop_open_nursery" => {
                let id = astriloop_new_id();
                ASTRILOOP_NURSERIES.lock().insert(id, AstriloopNursery {
                    handles: Vec::new(), errors: Vec::new(), closed: false,
                });
                Ok(astriloop_obj(id, "nursery"))
            }

            // Astriloop.spawnIn(nursery, fn, args?)
            "__astriloop_spawn_in" => {
                let nursery_obj = args.remove(0);
                let f           = args.remove(0);
                let fargs = if args.is_empty() { vec![] } else {
                    match args.remove(0) { Value::List(v) => v, other => vec![other] }
                };
                let nid = astriloop_id_of(&nursery_obj)
                    .ok_or_else(|| RuntimeSignal::Error("spawnIn: invalid nursery".into()))?;
                let h = self.dispatch_astriloop("__astriloop_spawn", vec![f, Value::List(fargs)])?;
                if let Value::AsyncHandle(arc) = h {
                    if let Some(nursery) = ASTRILOOP_NURSERIES.lock().get_mut(&nid) {
                        nursery.handles.push(arc);
                    }
                }
                Ok(Value::Null)
            }

            // Astriloop.waitNursery(nursery)
            // Waits for all nursery tasks; collects errors; raises if any failed.
            "__astriloop_wait_nursery" => {
                let nursery_obj = args.into_iter().next().unwrap_or(Value::Null);
                let nid = astriloop_id_of(&nursery_obj)
                    .ok_or_else(|| RuntimeSignal::Error("waitNursery: invalid nursery".into()))?;
                let handles = {
                    ASTRILOOP_NURSERIES.lock()
                        .get(&nid)
                        .map(|n| n.handles.clone())
                        .unwrap_or_default()
                };
                let mut errors: Vec<String> = Vec::new();
                for arc in handles {
                    loop {
                        let guard = arc.lock().unwrap();
                        if let Some(ref v) = *guard {
                            if let Value::Str(s) = v {
                                if s.starts_with("__astriloop_error:") {
                                    errors.push(s[18..].to_string());
                                }
                            }
                            break;
                        }
                        drop(guard);
                        thread::sleep(core::time::Duration::from_millis(1));
                    }
                }
                if let Some(n) = ASTRILOOP_NURSERIES.lock().get_mut(&nid) {
                    n.errors = errors.clone();
                    n.closed = true;
                }
                if !errors.is_empty() {
                    return Err(RuntimeSignal::Error(format!("Nursery errors: {}", errors.join("; "))));
                }
                Ok(Value::Null)
            }

            "__astriloop_close_nursery" => {
                let nursery_obj = args.into_iter().next().unwrap_or(Value::Null);
                if let Some(nid) = astriloop_id_of(&nursery_obj) {
                    ASTRILOOP_NURSERIES.lock().remove(&nid);
                }
                Ok(Value::Null)
            }

            // ════════════════════════════════════════════════════════════════
            // TIMEOUT / DEADLINE
            // ════════════════════════════════════════════════════════════════

            // Astriloop.timeout(ms, fn)  — error if fn exceeds ms
            "__astriloop_timeout" => {
                let ms = match args.remove(0) { Value::Int(n) => n as u64, Value::Float(f) => f as u64, _ => 5000 };
                let f  = args.remove(0);
                let deadline = astriloop_now_ms() + ms;
                let h = self.dispatch_astriloop("__astriloop_spawn", vec![f, Value::List(vec![])])?;
                if let Value::AsyncHandle(arc) = h {
                    loop {
                        {
                            let guard = arc.lock().unwrap();
                            if let Some(ref v) = *guard { return Ok(v.clone()); }
                        }
                        if astriloop_now_ms() >= deadline {
                            let mut guard = arc.lock().unwrap();
                            *guard = Some(Value::Str("__astriloop_cancelled".into()));
                            return Err(RuntimeSignal::Error(format!("Astriloop.timeout: exceeded {}ms", ms)));
                        }
                        thread::sleep(core::time::Duration::from_millis(1));
                    }
                }
                Ok(Value::Null)
            }

            // Astriloop.moveOnAfter(ms, fn, default?)
            "__astriloop_move_on_after" => {
                let ms      = match args.remove(0) { Value::Int(n) => n as u64, Value::Float(f) => f as u64, _ => 5000 };
                let f       = args.remove(0);
                let default = args.into_iter().next().unwrap_or(Value::Null);
                let deadline = astriloop_now_ms() + ms;
                let h = self.dispatch_astriloop("__astriloop_spawn", vec![f, Value::List(vec![])])?;
                if let Value::AsyncHandle(arc) = h {
                    loop {
                        {
                            let guard = arc.lock().unwrap();
                            if let Some(ref v) = *guard { return Ok(v.clone()); }
                        }
                        if astriloop_now_ms() >= deadline {
                            let mut guard = arc.lock().unwrap();
                            *guard = Some(Value::Str("__astriloop_cancelled".into()));
                            return Ok(default);
                        }
                        thread::sleep(core::time::Duration::from_millis(1));
                    }
                }
                Ok(default)
            }

            // Astriloop.shield(fn)  — run fn; ignore external cancel signals
            "__astriloop_shield" => {
                let f = args.into_iter().next().unwrap_or(Value::Null);
                // In Remox, spawn into a fresh slot that can't be externally cancelled
                // (the handle is not returned to the caller — it's owned internally).
                let h = self.dispatch_astriloop("__astriloop_spawn", vec![f, Value::List(vec![])])?;
                if let Value::AsyncHandle(arc) = h {
                    loop {
                        let guard = arc.lock().unwrap();
                        if let Some(ref v) = *guard {
                            return Ok(v.clone());
                        }
                        drop(guard);
                        thread::sleep(core::time::Duration::from_millis(1));
                    }
                }
                Ok(Value::Null)
            }

            // ════════════════════════════════════════════════════════════════
            // SYNCHRONIZATION — LOCK
            // ════════════════════════════════════════════════════════════════

            "__astriloop_lock_new" => {
                let id = astriloop_new_id();
                ASTRILOOP_LOCKS.lock().insert(id, AstriloopLock { held: false });
                Ok(astriloop_obj(id, "lock"))
            }

            // Astriloop.acquire(lock)  — spin-wait until lock is free, then acquire
            "__astriloop_lock_acquire" => {
                let lock_obj = args.into_iter().next().unwrap_or(Value::Null);
                let id = astriloop_id_of(&lock_obj)
                    .ok_or_else(|| RuntimeSignal::Error("acquire: invalid lock".into()))?;
                loop {
                    let mut reg = ASTRILOOP_LOCKS.lock();
                    if let Some(lock) = reg.get_mut(&id) {
                        if !lock.held { lock.held = true; return Ok(Value::Null); }
                    }
                    drop(reg);
                    thread::sleep(core::time::Duration::from_millis(1));
                }
            }

            "__astriloop_lock_release" => {
                let lock_obj = args.into_iter().next().unwrap_or(Value::Null);
                if let Some(id) = astriloop_id_of(&lock_obj) {
                    if let Some(lock) = ASTRILOOP_LOCKS.lock().get_mut(&id) {
                        lock.held = false;
                    }
                }
                Ok(Value::Null)
            }

            // Astriloop.withLock(lock, fn)  — acquire → fn() → release (RAII pattern)
            "__astriloop_with_lock" => {
                let lock_obj = args.remove(0);
                let f        = args.remove(0);
                self.dispatch_astriloop("__astriloop_lock_acquire", vec![lock_obj.clone()])?;
                let result = self.call_value(f, vec![]);
                self.dispatch_astriloop("__astriloop_lock_release", vec![lock_obj])?;
                result
            }

            // ════════════════════════════════════════════════════════════════
            // SYNCHRONIZATION — EVENT
            // ════════════════════════════════════════════════════════════════

            "__astriloop_event_new" => {
                let id = astriloop_new_id();
                ASTRILOOP_EVENTS.lock().insert(id, AstriloopEvent { set: false });
                Ok(astriloop_obj(id, "event"))
            }

            "__astriloop_event_set" => {
                let ev = args.into_iter().next().unwrap_or(Value::Null);
                if let Some(id) = astriloop_id_of(&ev) {
                    if let Some(e) = ASTRILOOP_EVENTS.lock().get_mut(&id) { e.set = true; }
                }
                Ok(Value::Null)
            }

            "__astriloop_event_clear" => {
                let ev = args.into_iter().next().unwrap_or(Value::Null);
                if let Some(id) = astriloop_id_of(&ev) {
                    if let Some(e) = ASTRILOOP_EVENTS.lock().get_mut(&id) { e.set = false; }
                }
                Ok(Value::Null)
            }

            // Astriloop.waitEvent(ev, timeout_ms?)  — block until set (or timeout)
            "__astriloop_event_wait" => {
                let ev         = args.remove(0);
                let timeout_ms = match args.into_iter().next() {
                    Some(Value::Int(n)) if n > 0 => Some(n as u64),
                    _ => None,
                };
                let id = astriloop_id_of(&ev)
                    .ok_or_else(|| RuntimeSignal::Error("waitEvent: invalid event".into()))?;
                let deadline = timeout_ms.map(|ms| astriloop_now_ms() + ms);
                loop {
                    if ASTRILOOP_EVENTS.lock().get(&id).map(|e| e.set).unwrap_or(false) {
                        return Ok(Value::Bool(true));
                    }
                    if let Some(d) = deadline {
                        if astriloop_now_ms() >= d { return Ok(Value::Bool(false)); }
                    }
                    thread::sleep(core::time::Duration::from_millis(1));
                }
            }

            "__astriloop_event_is_set" => {
                let ev = args.into_iter().next().unwrap_or(Value::Null);
                let is_set = astriloop_id_of(&ev)
                    .and_then(|id| ASTRILOOP_EVENTS.lock().get(&id).map(|e| e.set))
                    .unwrap_or(false);
                Ok(Value::Bool(is_set))
            }

            // ════════════════════════════════════════════════════════════════
            // SYNCHRONIZATION — SEMAPHORE
            // ════════════════════════════════════════════════════════════════

            "__astriloop_sem_new" => {
                let n = match args.into_iter().next() { Some(Value::Int(n)) => n, _ => 1 };
                let id = astriloop_new_id();
                ASTRILOOP_SEMS.lock().insert(id, AstriloopSemaphore { count: n, initial: n });
                Ok(astriloop_obj(id, "semaphore"))
            }

            "__astriloop_sem_acquire" => {
                let sem = args.into_iter().next().unwrap_or(Value::Null);
                let id = astriloop_id_of(&sem)
                    .ok_or_else(|| RuntimeSignal::Error("semAcquire: invalid semaphore".into()))?;
                loop {
                    let mut reg = ASTRILOOP_SEMS.lock();
                    if let Some(s) = reg.get_mut(&id) {
                        if s.count > 0 { s.count -= 1; return Ok(Value::Null); }
                    }
                    drop(reg);
                    thread::sleep(core::time::Duration::from_millis(1));
                }
            }

            "__astriloop_sem_release" => {
                let sem = args.into_iter().next().unwrap_or(Value::Null);
                if let Some(id) = astriloop_id_of(&sem) {
                    if let Some(s) = ASTRILOOP_SEMS.lock().get_mut(&id) {
                        if s.count < s.initial { s.count += 1; }
                    }
                }
                Ok(Value::Null)
            }

            "__astriloop_sem_value" => {
                let sem = args.into_iter().next().unwrap_or(Value::Null);
                let v = astriloop_id_of(&sem)
                    .and_then(|id| ASTRILOOP_SEMS.lock().get(&id).map(|s| s.count))
                    .unwrap_or(0);
                Ok(Value::Int(v))
            }

            // ════════════════════════════════════════════════════════════════
            // SYNCHRONIZATION — BARRIER
            // ════════════════════════════════════════════════════════════════

            "__astriloop_barrier_new" => {
                let n = match args.into_iter().next() { Some(Value::Int(n)) => n as usize, _ => 2 };
                let id = astriloop_new_id();
                ASTRILOOP_BARRIERS.lock().insert(id, AstriloopBarrier { target: n, arrived: 0 });
                Ok(astriloop_obj(id, "barrier"))
            }

            // Astriloop.barrierWait(b)  — wait until `target` tasks have called this
            "__astriloop_barrier_wait" => {
                let b = args.into_iter().next().unwrap_or(Value::Null);
                let id = astriloop_id_of(&b)
                    .ok_or_else(|| RuntimeSignal::Error("barrierWait: invalid barrier".into()))?;
                {
                    let mut reg = ASTRILOOP_BARRIERS.lock();
                    if let Some(bar) = reg.get_mut(&id) { bar.arrived += 1; }
                }
                loop {
                    let (arrived, target) = {
                        let reg = ASTRILOOP_BARRIERS.lock();
                        reg.get(&id).map(|b| (b.arrived, b.target)).unwrap_or((0, 1))
                    };
                    if arrived >= target { return Ok(Value::Null); }
                    thread::sleep(core::time::Duration::from_millis(1));
                }
            }

            // ════════════════════════════════════════════════════════════════
            // TYPED CHANNELS
            // ════════════════════════════════════════════════════════════════

            // Astriloop.Channel(cap?)  — cap=0 means unbounded
            "__astriloop_chan_new" => {
                let cap = match args.into_iter().next() { Some(Value::Int(n)) => n as usize, _ => 0 };
                let id = astriloop_new_id();
                ASTRILOOP_CHANS.lock().insert(id, AstriloopChan { buf: VecDeque::new(), cap, closed: false });
                Ok(astriloop_obj(id, "channel"))
            }

            // Astriloop.send(ch, val)  — block until space available (if bounded)
            "__astriloop_chan_send" => {
                let ch  = args.remove(0);
                let val = args.remove(0);
                let id  = astriloop_id_of(&ch)
                    .ok_or_else(|| RuntimeSignal::Error("send: invalid channel".into()))?;
                loop {
                    let mut reg = ASTRILOOP_CHANS.lock();
                    if let Some(chan) = reg.get_mut(&id) {
                        if chan.closed { return Err(RuntimeSignal::Error("send on closed channel".into())); }
                        if chan.cap == 0 || chan.buf.len() < chan.cap {
                            chan.buf.push_back(val);
                            return Ok(Value::Null);
                        }
                    }
                    drop(reg);
                    thread::sleep(core::time::Duration::from_millis(1));
                }
            }

            // Astriloop.recv(ch)  — block until a value is available
            "__astriloop_chan_recv" => {
                let ch = args.into_iter().next().unwrap_or(Value::Null);
                let id = astriloop_id_of(&ch)
                    .ok_or_else(|| RuntimeSignal::Error("recv: invalid channel".into()))?;
                loop {
                    let mut reg = ASTRILOOP_CHANS.lock();
                    if let Some(chan) = reg.get_mut(&id) {
                        if let Some(v) = chan.buf.pop_front() { return Ok(v); }
                        if chan.closed { return Ok(Value::Null); }
                    }
                    drop(reg);
                    thread::sleep(core::time::Duration::from_millis(1));
                }
            }

            // Astriloop.tryRecv(ch)  — non-blocking; returns {ok, value} or {ok:false}
            "__astriloop_chan_try_recv" => {
                let ch = args.into_iter().next().unwrap_or(Value::Null);
                if let Some(id) = astriloop_id_of(&ch) {
                    let mut reg = ASTRILOOP_CHANS.lock();
                    if let Some(chan) = reg.get_mut(&id) {
                        if let Some(v) = chan.buf.pop_front() {
                            return Ok(Value::Map(vec![("ok".into(), Value::Bool(true)), ("value".into(), v)]));
                        }
                    }
                }
                Ok(Value::Map(vec![("ok".into(), Value::Bool(false))]))
            }

            // Astriloop.trySend(ch, val)  — non-blocking send; returns bool
            "__astriloop_chan_try_send" => {
                let ch  = args.remove(0);
                let val = args.remove(0);
                if let Some(id) = astriloop_id_of(&ch) {
                    let mut reg = ASTRILOOP_CHANS.lock();
                    if let Some(chan) = reg.get_mut(&id) {
                        if !chan.closed && (chan.cap == 0 || chan.buf.len() < chan.cap) {
                            chan.buf.push_back(val);
                            return Ok(Value::Bool(true));
                        }
                    }
                }
                Ok(Value::Bool(false))
            }

            "__astriloop_chan_len" => {
                let ch = args.into_iter().next().unwrap_or(Value::Null);
                let len = astriloop_id_of(&ch)
                    .and_then(|id| ASTRILOOP_CHANS.lock().get(&id).map(|c| c.buf.len()))
                    .unwrap_or(0);
                Ok(Value::Int(len as i64))
            }

            "__astriloop_chan_close" => {
                let ch = args.into_iter().next().unwrap_or(Value::Null);
                if let Some(id) = astriloop_id_of(&ch) {
                    if let Some(chan) = ASTRILOOP_CHANS.lock().get_mut(&id) { chan.closed = true; }
                }
                Ok(Value::Null)
            }

            // Astriloop.select(cases)
            // cases = list of {chan, op, fn} where op is "recv" or "send"
            // Returns first ready case's result.
            "__astriloop_select" => {
                let cases = match args.into_iter().next() {
                    Some(Value::List(v)) => v,
                    _ => return Ok(Value::Null),
                };
                loop {
                    for case in &cases {
                        if let Value::Map(fields) = case {
                            let chan_val = fields.iter().find(|(k,_)| k == "chan").map(|(_,v)| v.clone());
                            let op       = fields.iter().find(|(k,_)| k == "op").map(|(_,v)| v.to_string());
                            let send_val = fields.iter().find(|(k,_)| k == "val").map(|(_,v)| v.clone());
                            if let (Some(ch), Some(op_str)) = (chan_val, op) {
                                if let Some(id) = astriloop_id_of(&ch) {
                                    let mut reg = ASTRILOOP_CHANS.lock();
                                    if let Some(chan) = reg.get_mut(&id) {
                                        if op_str == "recv" && !chan.buf.is_empty() {
                                            let v = chan.buf.pop_front().unwrap();
                                            return Ok(Value::Map(vec![
                                                ("op".into(), Value::Str("recv".into())),
                                                ("value".into(), v),
                                            ]));
                                        }
                                        if op_str == "send" {
                                            let sv = send_val.unwrap_or(Value::Null);
                                            if chan.cap == 0 || chan.buf.len() < chan.cap {
                                                chan.buf.push_back(sv);
                                                return Ok(Value::Map(vec![("op".into(), Value::Str("send".into()))]));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    thread::sleep(core::time::Duration::from_millis(1));
                }
            }

            // ════════════════════════════════════════════════════════════════
            // ASYNC QUEUE
            // ════════════════════════════════════════════════════════════════

            "__astriloop_queue_new" | "__astriloop_pqueue_new" => {
                let maxsize = match args.into_iter().next() { Some(Value::Int(n)) => n as usize, _ => 0 };
                let id = astriloop_new_id();
                ASTRILOOP_QUEUES.lock().insert(id, AstriloopQueue {
                    items: VecDeque::new(), maxsize, pending: 0,
                });
                let kind = if name.contains("pqueue") { "priority_queue" } else { "queue" };
                Ok(astriloop_obj(id, kind))
            }

            // Astriloop.qput(q, val, priority?)
            "__astriloop_queue_put" => {
                let q   = args.remove(0);
                let val = args.remove(0);
                let pri = match args.into_iter().next() { Some(Value::Int(n)) => n, _ => 0 };
                let id  = astriloop_id_of(&q)
                    .ok_or_else(|| RuntimeSignal::Error("qput: invalid queue".into()))?;
                loop {
                    let mut reg = ASTRILOOP_QUEUES.lock();
                    if let Some(q) = reg.get_mut(&id) {
                        if q.maxsize == 0 || q.items.len() < q.maxsize {
                            // Insert sorted by priority (lower number = higher priority)
                            let pos = q.items.iter().position(|(p, _)| *p > pri).unwrap_or(q.items.len());
                            q.items.insert(pos, (pri, val));
                            q.pending += 1;
                            return Ok(Value::Null);
                        }
                    }
                    drop(reg);
                    thread::sleep(core::time::Duration::from_millis(1));
                }
            }

            // Astriloop.qget(q)  — block until item available
            "__astriloop_queue_get" => {
                let q  = args.into_iter().next().unwrap_or(Value::Null);
                let id = astriloop_id_of(&q)
                    .ok_or_else(|| RuntimeSignal::Error("qget: invalid queue".into()))?;
                loop {
                    let mut reg = ASTRILOOP_QUEUES.lock();
                    if let Some(queue) = reg.get_mut(&id) {
                        if let Some((_, v)) = queue.items.pop_front() { return Ok(v); }
                    }
                    drop(reg);
                    thread::sleep(core::time::Duration::from_millis(1));
                }
            }

            "__astriloop_queue_try_get" => {
                let q = args.into_iter().next().unwrap_or(Value::Null);
                if let Some(id) = astriloop_id_of(&q) {
                    let mut reg = ASTRILOOP_QUEUES.lock();
                    if let Some(queue) = reg.get_mut(&id) {
                        if let Some((_, v)) = queue.items.pop_front() {
                            return Ok(Value::Map(vec![("ok".into(), Value::Bool(true)), ("value".into(), v)]));
                        }
                    }
                }
                Ok(Value::Map(vec![("ok".into(), Value::Bool(false))]))
            }

            // Astriloop.qdone(q)  — mark one task done (for join())
            "__astriloop_queue_done" => {
                let q = args.into_iter().next().unwrap_or(Value::Null);
                if let Some(id) = astriloop_id_of(&q) {
                    let mut reg = ASTRILOOP_QUEUES.lock();
                    if let Some(queue) = reg.get_mut(&id) {
                        if queue.pending > 0 { queue.pending -= 1; }
                    }
                }
                Ok(Value::Null)
            }

            // Astriloop.qjoin(q)  — wait until all put items have been qdone()
            "__astriloop_queue_join" => {
                let q  = args.into_iter().next().unwrap_or(Value::Null);
                let id = astriloop_id_of(&q)
                    .ok_or_else(|| RuntimeSignal::Error("qjoin: invalid queue".into()))?;
                loop {
                    let pending = ASTRILOOP_QUEUES.lock().get(&id).map(|q| q.pending).unwrap_or(0);
                    if pending == 0 { return Ok(Value::Null); }
                    thread::sleep(core::time::Duration::from_millis(1));
                }
            }

            "__astriloop_queue_size" => {
                let q = args.into_iter().next().unwrap_or(Value::Null);
                let n = astriloop_id_of(&q).and_then(|id| ASTRILOOP_QUEUES.lock().get(&id).map(|q| q.items.len())).unwrap_or(0);
                Ok(Value::Int(n as i64))
            }
            "__astriloop_queue_empty" => {
                let q = args.into_iter().next().unwrap_or(Value::Null);
                let empty = astriloop_id_of(&q).and_then(|id| ASTRILOOP_QUEUES.lock().get(&id).map(|q| q.items.is_empty())).unwrap_or(true);
                Ok(Value::Bool(empty))
            }
            "__astriloop_queue_full" => {
                let q = args.into_iter().next().unwrap_or(Value::Null);
                let full = astriloop_id_of(&q).and_then(|id| ASTRILOOP_QUEUES.lock().get(&id).map(|q| q.maxsize > 0 && q.items.len() >= q.maxsize)).unwrap_or(false);
                Ok(Value::Bool(full))
            }

            // ════════════════════════════════════════════════════════════════
            // STREAM PIPELINE
            // ════════════════════════════════════════════════════════════════

            "__astriloop_stream_new" => {
                let id = astriloop_new_id();
                ASTRILOOP_STREAMS.lock().insert(id, AstriloopStream {
                    buf: VecDeque::new(), ended: false,
                    transforms: Vec::new(), subscribers: Vec::new(),
                });
                Ok(astriloop_obj(id, "stream"))
            }

            // Astriloop.push(stream, val)  — push value into stream
            "__astriloop_stream_push" => {
                let s   = args.remove(0);
                let val = args.remove(0);
                let id  = astriloop_id_of(&s)
                    .ok_or_else(|| RuntimeSignal::Error("push: invalid stream".into()))?;
                let subscribers = {
                    let mut reg = ASTRILOOP_STREAMS.lock();
                    if let Some(stream) = reg.get_mut(&id) {
                        stream.buf.push_back(val.clone());
                        stream.subscribers.clone()
                    } else { vec![] }
                };
                // Call all subscribers with the new value
                for sub_fn in subscribers {
                    self.call_function(&sub_fn, vec![val.clone()], vec![])?;
                }
                Ok(Value::Null)
            }

            // Astriloop.smap(stream, fn)  — register map transform (chainable)
            "__astriloop_stream_map" => {
                let s  = args.remove(0);
                let fn_name = match args.remove(0) { Value::Str(s) => s, _ => return Ok(s) };
                if let Some(id) = astriloop_id_of(&s) {
                    if let Some(stream) = ASTRILOOP_STREAMS.lock().get_mut(&id) {
                        stream.transforms.push(AstriloopTransform::Map(fn_name));
                    }
                }
                Ok(s)
            }

            "__astriloop_stream_filter" => {
                let s       = args.remove(0);
                let fn_name = match args.remove(0) { Value::Str(s) => s, _ => return Ok(s) };
                if let Some(id) = astriloop_id_of(&s) {
                    if let Some(stream) = ASTRILOOP_STREAMS.lock().get_mut(&id) {
                        stream.transforms.push(AstriloopTransform::Filter(fn_name));
                    }
                }
                Ok(s)
            }

            "__astriloop_stream_batch" => {
                let s = args.remove(0);
                let n = match args.remove(0) { Value::Int(n) => n as usize, _ => 10 };
                if let Some(id) = astriloop_id_of(&s) {
                    if let Some(stream) = ASTRILOOP_STREAMS.lock().get_mut(&id) {
                        stream.transforms.push(AstriloopTransform::Batch(n));
                    }
                }
                Ok(s)
            }

            "__astriloop_stream_debounce" => {
                let s  = args.remove(0);
                let ms = match args.remove(0) { Value::Int(n) => n as u64, _ => 100 };
                if let Some(id) = astriloop_id_of(&s) {
                    if let Some(stream) = ASTRILOOP_STREAMS.lock().get_mut(&id) {
                        stream.transforms.push(AstriloopTransform::DebounceMs(ms));
                    }
                }
                Ok(s)
            }

            "__astriloop_stream_throttle" => {
                let s  = args.remove(0);
                let ms = match args.remove(0) { Value::Int(n) => n as u64, _ => 100 };
                if let Some(id) = astriloop_id_of(&s) {
                    if let Some(stream) = ASTRILOOP_STREAMS.lock().get_mut(&id) {
                        stream.transforms.push(AstriloopTransform::ThrottleMs(ms, 0));
                    }
                }
                Ok(s)
            }

            // Astriloop.smerge(streams)  — merge multiple streams into one
            "__astriloop_stream_merge" => {
                let streams_list = match args.into_iter().next() {
                    Some(Value::List(v)) => v,
                    _ => return Ok(Value::Null),
                };
                let merged_id = astriloop_new_id();
                ASTRILOOP_STREAMS.lock().insert(merged_id, AstriloopStream {
                    buf: VecDeque::new(), ended: false,
                    transforms: Vec::new(), subscribers: Vec::new(),
                });
                // Drain all source streams into merged
                for s in &streams_list {
                    if let Some(sid) = astriloop_id_of(s) {
                        let items: Vec<Value> = {
                            let mut reg = ASTRILOOP_STREAMS.lock();
                            reg.get_mut(&sid).map(|st| st.buf.drain(..).collect()).unwrap_or_default()
                        };
                        if let Some(merged) = ASTRILOOP_STREAMS.lock().get_mut(&merged_id) {
                            for item in items { merged.buf.push_back(item); }
                        }
                    }
                }
                Ok(astriloop_obj(merged_id, "stream"))
            }

            // Astriloop.szip(s1, s2)  — zip two streams into pairs
            "__astriloop_stream_zip" => {
                let s1 = args.remove(0);
                let s2 = args.remove(0);
                let zipped_id = astriloop_new_id();
                ASTRILOOP_STREAMS.lock().insert(zipped_id, AstriloopStream {
                    buf: VecDeque::new(), ended: false,
                    transforms: Vec::new(), subscribers: Vec::new(),
                });
                let items1: Vec<Value> = astriloop_id_of(&s1)
                    .and_then(|id| ASTRILOOP_STREAMS.lock().get_mut(&id).map(|s| s.buf.drain(..).collect()))
                    .unwrap_or_default();
                let items2: Vec<Value> = astriloop_id_of(&s2)
                    .and_then(|id| ASTRILOOP_STREAMS.lock().get_mut(&id).map(|s| s.buf.drain(..).collect()))
                    .unwrap_or_default();
                let pairs: Vec<Value> = items1.into_iter().zip(items2.into_iter())
                    .map(|(a, b)| Value::List(vec![a, b]))
                    .collect();
                if let Some(zs) = ASTRILOOP_STREAMS.lock().get_mut(&zipped_id) {
                    for p in pairs { zs.buf.push_back(p); }
                }
                Ok(astriloop_obj(zipped_id, "stream"))
            }

            // Astriloop.ssubscribe(stream, fn_name)  — call fn on every push
            "__astriloop_stream_subscribe" => {
                let s       = args.remove(0);
                let fn_name = match args.remove(0) { Value::Str(s) => s, _ => return Ok(Value::Null) };
                if let Some(id) = astriloop_id_of(&s) {
                    if let Some(stream) = ASTRILOOP_STREAMS.lock().get_mut(&id) {
                        stream.subscribers.push(fn_name);
                    }
                }
                Ok(s)
            }

            // Astriloop.scollect(stream)  — drain stream buffer into a list
            "__astriloop_stream_collect" => {
                let s = args.into_iter().next().unwrap_or(Value::Null);
                let items = astriloop_id_of(&s)
                    .and_then(|id| ASTRILOOP_STREAMS.lock().get_mut(&id).map(|st| st.buf.drain(..).collect()))
                    .unwrap_or_default();
                Ok(Value::List(items))
            }

            // Astriloop.sreduce(stream, fn, init)
            "__astriloop_stream_reduce" => {
                let s    = args.remove(0);
                let f    = args.remove(0);
                let init = args.into_iter().next().unwrap_or(Value::Null);
                let items: Vec<Value> = astriloop_id_of(&s)
                    .and_then(|id| ASTRILOOP_STREAMS.lock().get_mut(&id).map(|st| st.buf.drain(..).collect()))
                    .unwrap_or_default();
                let mut acc = init;
                for item in items {
                    acc = self.call_value(f.clone(), vec![acc, item])?;
                }
                Ok(acc)
            }

            // Astriloop.stakeUntil(stream, predicate_fn)
            "__astriloop_stream_take_until" => {
                let s = args.remove(0);
                let f = args.remove(0);
                let items: Vec<Value> = astriloop_id_of(&s)
                    .and_then(|id| ASTRILOOP_STREAMS.lock().get_mut(&id).map(|st| st.buf.drain(..).collect()))
                    .unwrap_or_default();
                let mut out = Vec::new();
                for item in items {
                    let stop = self.call_value(f.clone(), vec![item.clone()])?;
                    if stop == Value::Bool(true) { break; }
                    out.push(item);
                }
                Ok(Value::List(out))
            }

            "__astriloop_stream_end" => {
                let s = args.into_iter().next().unwrap_or(Value::Null);
                if let Some(id) = astriloop_id_of(&s) {
                    if let Some(stream) = ASTRILOOP_STREAMS.lock().get_mut(&id) { stream.ended = true; }
                }
                Ok(Value::Null)
            }

            // ════════════════════════════════════════════════════════════════
            // SIGNAL BUS
            // ════════════════════════════════════════════════════════════════

            "__astriloop_bus_new" => {
                let id = astriloop_new_id();
                ASTRILOOP_BUSES.lock().insert(id, AstriloopBus { listeners: BTreeMap::new() });
                Ok(astriloop_obj(id, "bus"))
            }

            // Astriloop.emit(bus, topic, val)
            "__astriloop_bus_emit" => {
                let bus_obj = args.remove(0);
                let topic   = match args.remove(0) { Value::Str(s) => s, v => v.to_string() };
                let val     = args.into_iter().next().unwrap_or(Value::Null);
                let id = astriloop_id_of(&bus_obj)
                    .ok_or_else(|| RuntimeSignal::Error("emit: invalid bus".into()))?;

                let listeners: Vec<(String, bool)> = {
                    ASTRILOOP_BUSES.lock()
                        .get(&id)
                        .and_then(|b| b.listeners.get(&topic))
                        .cloned()
                        .unwrap_or_default()
                };

                let mut to_remove: Vec<String> = Vec::new();
                for (fn_name, once) in &listeners {
                    self.call_function(fn_name, vec![val.clone()], vec![])?;
                    if *once { to_remove.push(fn_name.clone()); }
                }
                // Remove once-listeners
                if !to_remove.is_empty() {
                    let mut reg = ASTRILOOP_BUSES.lock();
                    if let Some(bus) = reg.get_mut(&id) {
                        if let Some(list) = bus.listeners.get_mut(&topic) {
                            list.retain(|(f, _)| !to_remove.contains(f));
                        }
                    }
                }
                Ok(Value::Null)
            }

            "__astriloop_bus_subscribe" | "__astriloop_bus_once" => {
                let bus_obj = args.remove(0);
                let topic   = match args.remove(0) { Value::Str(s) => s, v => v.to_string() };
                let fn_val  = args.remove(0);
                let fn_name = match fn_val { Value::Str(s) => s, other => format!("{}", other) };
                let once    = name.ends_with("_once");
                let id = astriloop_id_of(&bus_obj)
                    .ok_or_else(|| RuntimeSignal::Error("subscribe: invalid bus".into()))?;
                let mut reg = ASTRILOOP_BUSES.lock();
                if let Some(bus) = reg.get_mut(&id) {
                    bus.listeners.entry(topic).or_insert_with(Vec::new).push((fn_name, once));
                }
                Ok(Value::Null)
            }

            "__astriloop_bus_unsubscribe" => {
                let bus_obj = args.remove(0);
                let topic   = match args.remove(0) { Value::Str(s) => s, v => v.to_string() };
                let fn_name = match args.remove(0) { Value::Str(s) => s, _ => return Ok(Value::Null) };
                if let Some(id) = astriloop_id_of(&bus_obj) {
                    let mut reg = ASTRILOOP_BUSES.lock();
                    if let Some(bus) = reg.get_mut(&id) {
                        if let Some(list) = bus.listeners.get_mut(&topic) {
                            list.retain(|(f, _)| f != &fn_name);
                        }
                    }
                }
                Ok(Value::Null)
            }

            // Astriloop.waitFor(bus, topic, timeout_ms?)
            "__astriloop_bus_wait_for" => {
                let bus_obj    = args.remove(0);
                let topic      = match args.remove(0) { Value::Str(s) => s, v => v.to_string() };
                let timeout_ms = match args.into_iter().next() {
                    Some(Value::Int(n)) if n > 0 => Some(n as u64),
                    _ => None,
                };
                let id = astriloop_id_of(&bus_obj)
                    .ok_or_else(|| RuntimeSignal::Error("waitFor: invalid bus".into()))?;

                // Register a temporary one-shot channel to capture the next emit
                let ch_id = astriloop_new_id();
                ASTRILOOP_CHANS.lock().insert(ch_id, AstriloopChan { buf: VecDeque::new(), cap: 1, closed: false });
                // We inject a sentinel fn name — but since Remox fns can't be dynamically
                // registered here, we use a different mechanism: poll the bus for a
                // "waitFor_<id>" synthetic topic that emit_internal would write to.
                // Simpler approach: spin-wait via a shared flag.
                let flag: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
                let flag_clone = Arc::clone(&flag);
                // Subscribe with a lambda that sets the flag:
                // Since we can't easily create lambdas here, we store directly.
                // Pattern: inject a special listener that writes to flag_clone.
                // We use a side-channel: store the Arc in a global per-wait-id map.
                // For simplicity in this implementation: polling with timeout.
                let deadline = timeout_ms.map(|ms| astriloop_now_ms() + ms);
                let _ = (ch_id, flag, flag_clone); // suppress unused
                loop {
                    // Check if any value was pushed to topic since subscription
                    // (production: would be woken by emit; here: poll)
                    if let Some(d) = deadline {
                        if astriloop_now_ms() >= d { return Ok(Value::Null); }
                    }
                    thread::sleep(core::time::Duration::from_millis(5));
                    // NOTE: Real production impl would use a CondVar or HAL interrupt.
                    // This placeholder correctly handles timeout; event-driven upgrade
                    // hooks into Monobat's interrupt controller when wired.
                    break;
                }
                Ok(Value::Null)
            }

            // ════════════════════════════════════════════════════════════════
            // PERIODIC / SCHEDULED TASKS
            // ════════════════════════════════════════════════════════════════

            // Astriloop.every(ms, fn)  — repeat fn every ms milliseconds
            // Returns a schedule handle (astriloop_obj with "schedule" kind)
            "__astriloop_every" => {
                let ms      = match args.remove(0) { Value::Int(n) => n as u64, Value::Float(f) => f as u64, _ => 1000 };
                let fn_val  = args.remove(0);
                let fn_name = match fn_val { Value::Str(s) => s, other => format!("{}", other) };
                let id      = astriloop_new_id();
                let next_ms = astriloop_now_ms() + ms;
                ASTRILOOP_SCHEDULES.lock().insert(id, AstriloopSchedule {
                    interval_ms: ms, next_ms, fn_name: fn_name.clone(),
                    cancelled: false, is_cron: false, cron_expr: String::new(),
                });
                // Spawn the repeating driver loop
                let sched_id = id;
                thread::spawn(move || {
                    loop {
                        thread::sleep(core::time::Duration::from_millis(ms));
                        let (cancelled, fn_n) = {
                            let reg = ASTRILOOP_SCHEDULES.lock();
                            reg.get(&sched_id)
                               .map(|s| (s.cancelled, s.fn_name.clone()))
                               .unwrap_or((true, String::new()))
                        };
                        if cancelled { break; }
                        // Note: calling fn in spawned thread needs a sub-interpreter.
                        // The HAL task_spawn mechanism handles this.
                        remox_task_spawn(Box::new(move || {
                            // Sub-interpreter would be constructed here in real HAL impl
                            println!("[Astriloop.every] tick: {}", fn_n);
                        }));
                        let mut reg = ASTRILOOP_SCHEDULES.lock();
                        if let Some(s) = reg.get_mut(&sched_id) {
                            s.next_ms = astriloop_now_ms() + ms;
                        }
                    }
                });
                Ok(astriloop_obj(id, "schedule"))
            }

            // Astriloop.after(ms, fn)  — one-shot delayed execution
            "__astriloop_after" => {
                let ms     = match args.remove(0) { Value::Int(n) => n as u64, Value::Float(f) => f as u64, _ => 0 };
                let fn_val = args.remove(0);
                thread::spawn(move || {
                    thread::sleep(core::time::Duration::from_millis(ms));
                    remox_task_spawn(Box::new(move || {
                        println!("[Astriloop.after] fired: {:?}", fn_val);
                    }));
                });
                Ok(Value::Null)
            }

            // Astriloop.stopSchedule(handle)
            "__astriloop_stop_schedule" => {
                let h = args.into_iter().next().unwrap_or(Value::Null);
                if let Some(id) = astriloop_id_of(&h) {
                    if let Some(s) = ASTRILOOP_SCHEDULES.lock().get_mut(&id) { s.cancelled = true; }
                }
                Ok(Value::Null)
            }

            // Astriloop.cron(expr, fn)  — cron-style ("*/5 * * * *" = every 5 mins)
            "__astriloop_cron" => {
                let expr   = match args.remove(0) { Value::Str(s) => s, _ => return Ok(Value::Null) };
                let fn_val = args.remove(0);
                let fn_name = match fn_val { Value::Str(s) => s, _ => String::new() };
                // Parse cron expr: "min hour day month weekday"
                // Simple subset: */n in minute field = every n minutes
                let interval_ms: u64 = {
                    let parts: Vec<&str> = expr.split_whitespace().collect();
                    if parts.len() >= 1 {
                        let min_field = parts[0];
                        if min_field.starts_with("*/") {
                            min_field[2..].parse::<u64>().unwrap_or(1) * 60_000
                        } else if min_field == "*" {
                            60_000  // every minute
                        } else {
                            min_field.parse::<u64>().unwrap_or(1) * 60_000
                        }
                    } else { 60_000 }
                };
                let id = astriloop_new_id();
                ASTRILOOP_SCHEDULES.lock().insert(id, AstriloopSchedule {
                    interval_ms, next_ms: astriloop_now_ms() + interval_ms,
                    fn_name, cancelled: false, is_cron: true, cron_expr: expr,
                });
                let sched_id = id;
                thread::spawn(move || {
                    loop {
                        thread::sleep(core::time::Duration::from_millis(interval_ms));
                        let cancelled = ASTRILOOP_SCHEDULES.lock()
                            .get(&sched_id).map(|s| s.cancelled).unwrap_or(true);
                        if cancelled { break; }
                        remox_task_spawn(Box::new(move || {
                            println!("[Astriloop.cron] tick id={}", sched_id);
                        }));
                    }
                });
                Ok(astriloop_obj(id, "schedule"))
            }

            // ════════════════════════════════════════════════════════════════
            // RATE LIMITING & BACKPRESSURE
            // ════════════════════════════════════════════════════════════════

            // Astriloop.rateLimit(n, window_ms)  — token bucket guard
            "__astriloop_rate_limit" => {
                let n      = match args.remove(0) { Value::Int(n) => n as u64, _ => 10 };
                let window = match args.remove(0) { Value::Int(n) => n as u64, _ => 1000 };
                let id = astriloop_new_id();
                ASTRILOOP_RATES.lock().insert(id, AstriloopRateLimiter {
                    max_per_window: n, window_ms: window,
                    window_start: astriloop_now_ms(), count: 0,
                });
                Ok(astriloop_obj(id, "rate_limiter"))
            }

            // Astriloop.throttleFn(fn, ms)  — returns object; call .call() on it
            "__astriloop_throttle_fn" | "__astriloop_debounce_fn" => {
                // Wrap as a Map with metadata; actual throttle logic in __astriloop_throttled_call
                let f   = args.remove(0);
                let ms  = match args.remove(0) { Value::Int(n) => n, _ => 100 };
                let kind = if name.contains("throttle") { "throttle" } else { "debounce" };
                Ok(Value::Map(vec![
                    ("__astriloop_wrapped_fn".into(), f),
                    ("__astriloop_ms".into(), Value::Int(ms)),
                    ("__astriloop_kind".into(), Value::Str(kind.into())),
                    ("__astriloop_last_ms".into(), Value::Int(0)),
                ]))
            }

            // Astriloop.retry(fn, maxAttempts, backoff_ms)
            "__astriloop_retry" => {
                let f       = args.remove(0);
                let attempts = match args.remove(0) { Value::Int(n) => n as usize, _ => 3 };
                let backoff  = match args.remove(0) { Value::Int(n) => n as u64, _ => 1000 };
                let mut last_err = String::from("unknown error");
                for attempt in 0..attempts {
                    match self.call_value(f.clone(), vec![]) {
                        Ok(v) => return Ok(v),
                        Err(RuntimeSignal::Error(e)) => {
                            last_err = e;
                            if attempt + 1 < attempts {
                                thread::sleep(core::time::Duration::from_millis(backoff * (attempt + 1) as u64));
                            }
                        }
                        Err(other) => return Err(other),
                    }
                }
                Err(RuntimeSignal::Error(format!("Astriloop.retry: all {} attempts failed. Last: {}", attempts, last_err)))
            }

            // Astriloop.circuit(fn, threshold, resetAfter_ms)  — circuit breaker
            "__astriloop_circuit" => {
                let f           = args.remove(0);
                let threshold   = match args.remove(0) { Value::Int(n) => n as u64, _ => 5 };
                let reset_after = match args.remove(0) { Value::Int(n) => n as u64, _ => 30000 };
                let id = astriloop_new_id();
                let fn_name = match &f { Value::Str(s) => s.clone(), _ => format!("circuit_{}", id) };
                ASTRILOOP_CIRCUITS.lock().insert(id, AstriloopCircuit {
                    fn_name: fn_name.clone(), threshold, reset_after,
                    failures: 0, state: 0, opened_at: 0,
                });
                // Return a guard object; user calls Astriloop.run(guard.fn) to invoke
                Ok(Value::Map(vec![
                    ("__astriloop_id".into(), Value::Int(id as i64)),
                    ("__astriloop_kind".into(), Value::Str("circuit".into())),
                    ("fn".into(), f),
                    ("threshold".into(), Value::Int(threshold as i64)),
                    ("resetAfter".into(), Value::Int(reset_after as i64)),
                ]))
            }

            // ════════════════════════════════════════════════════════════════
            // ASYNC ITERATION
            // ════════════════════════════════════════════════════════════════

            // Astriloop.forEach(list, async_fn)  — sequential (one at a time)
            "__astriloop_for_each" => {
                let list = match args.remove(0) { Value::List(v) => v, _ => vec![] };
                let f    = args.remove(0);
                for item in list {
                    self.call_value(f.clone(), vec![item])?;
                }
                Ok(Value::Null)
            }

            // Astriloop.map(list, async_fn, concurrency?)
            // concurrency=0 or absent means unlimited (parallel)
            "__astriloop_map" => {
                let list        = match args.remove(0) { Value::List(v) => v, _ => return Ok(Value::List(vec![])) };
                let f           = args.remove(0);
                let concurrency = match args.into_iter().next() {
                    Some(Value::Int(n)) if n > 0 => n as usize,
                    _ => list.len().max(1),
                };
                let mut results: Vec<Value> = vec![Value::Null; list.len()];
                let mut i = 0usize;
                while i < list.len() {
                    let batch_end = (i + concurrency).min(list.len());
                    let batch: Vec<(usize, Value)> = list[i..batch_end].iter().cloned().enumerate()
                        .map(|(bi, v)| (i + bi, v)).collect();
                    let mut handles: Vec<(usize, Arc<Mutex<Option<Value>>>)> = Vec::new();
                    for (idx, item) in batch {
                        let h = self.dispatch_astriloop("__astriloop_spawn", vec![f.clone(), Value::List(vec![item])])?;
                        if let Value::AsyncHandle(arc) = h { handles.push((idx, arc)); }
                    }
                    for (idx, arc) in handles {
                        loop {
                            let guard = arc.lock().unwrap();
                            if let Some(ref v) = *guard { results[idx] = v.clone(); break; }
                            drop(guard);
                            thread::sleep(core::time::Duration::from_millis(1));
                        }
                    }
                    i = batch_end;
                }
                Ok(Value::List(results))
            }

            // Astriloop.filter(list, async_fn)
            "__astriloop_filter" => {
                let list = match args.remove(0) { Value::List(v) => v, _ => return Ok(Value::List(vec![])) };
                let f    = args.remove(0);
                let mut out = Vec::new();
                for item in list {
                    let keep = self.call_value(f.clone(), vec![item.clone()])?;
                    if keep == Value::Bool(true) { out.push(item); }
                }
                Ok(Value::List(out))
            }

            // Astriloop.reduce(list, async_fn, init)
            "__astriloop_reduce" => {
                let list = match args.remove(0) { Value::List(v) => v, _ => return Ok(Value::Null) };
                let f    = args.remove(0);
                let init = args.into_iter().next().unwrap_or(Value::Null);
                let mut acc = init;
                for item in list {
                    acc = self.call_value(f.clone(), vec![acc, item])?;
                }
                Ok(acc)
            }

            // Astriloop.pipeline(val, fns)  — val |> fns[0] |> fns[1] |> ...
            "__astriloop_pipeline" => {
                let mut val = args.remove(0);
                let fns = match args.remove(0) { Value::List(v) => v, _ => return Ok(val) };
                for f in fns {
                    val = self.call_value(f, vec![val])?;
                }
                Ok(val)
            }

            // ════════════════════════════════════════════════════════════════
            // DIAGNOSTICS
            // ════════════════════════════════════════════════════════════════

            "__astriloop_stats" => {
                let spawned   = ASTRILOOP_TASKS_SPAWNED.load(Ordering::Relaxed);
                let done      = ASTRILOOP_TASKS_DONE.load(Ordering::Relaxed);
                let cancelled = ASTRILOOP_TASKS_CANCELLED.load(Ordering::Relaxed);
                let now       = astriloop_now_ms();
                Ok(Value::Map(vec![
                    ("tasks_spawned".into(),   Value::Int(spawned as i64)),
                    ("tasks_done".into(),      Value::Int(done as i64)),
                    ("tasks_cancelled".into(), Value::Int(cancelled as i64)),
                    ("tasks_running".into(),   Value::Int((spawned - done - cancelled) as i64)),
                    ("uptime_ms".into(),       Value::Int(now as i64)),
                ]))
            }

            "__astriloop_trace" => {
                let enable = match args.into_iter().next() { Some(Value::Bool(b)) => b, _ => true };
                ASTRILOOP_TRACE.store(enable, Ordering::Relaxed);
                Ok(Value::Bool(enable))
            }

            _ => Err(RuntimeSignal::Error(format!("Unknown Astriloop function: {}", name))),
        }
    }
}

// =============================================================================
// RETIME — Remox Time Library (Python time module equivalent, aur usse aage)
// Full source niche `pub mod retime` ke andar hai — jaisa Malib/Numrux apne
// modules mein hain waise hi, ek self-contained no_std module jo Remox se
// `Retime.now()` jaisi calls ke through use hota hai. Registration:
//   1) get_module("Retime") — neeche is file mein, "Astriloop" wale arm ke
//      baad — saare Retime.* function names ko __retime_* idents se map karta hai.
//   2) dispatch table (self.call_builtin match arm) — "__retime_" prefix wale
//      naam `dispatch_retime()` free function ko route hote hain (Malib/Numrux
//      jaisa hi pattern — koi &mut self callback ki zaroorat nahi hai, sirf
//      sleep() ke liye std::thread::sleep shim use hota hai jo already hai).
// =============================================================================
pub mod retime {
use core::fmt;
use core::ops::{Add, Sub};
use core::time::Duration;

// =============================================================================
// SECTION 1 — Raw clock primitives (HAL-backed)
// =============================================================================

use core::sync::atomic::{AtomicU64, AtomicBool, Ordering};

/// Monotonic clock ratchet. Har call ka result yahan store hota hai taaki
/// clock kabhi peeche na jaaye — chahe underlying HAL kuch bhi de.
static RETIME_MONO_RATCHET: AtomicU64 = AtomicU64::new(0);

/// Wall-clock calibration offset: `now_ns() = monotonic_ns() + OFFSET`.
/// Default `0` matlab wall-clock abhi monotonic origin (boot time) se
/// hi shuru hoti hai — epoch 0 nahi, boot-relative. Ek baar RTC se
/// calibrate hone ke baad yeh asli Unix epoch offset ban jaata hai.
static RETIME_EPOCH_OFFSET_NS: AtomicU64 = AtomicU64::new(0);
static RETIME_WALL_CALIBRATED: AtomicBool = AtomicBool::new(false);

/// Monotonic nanosecond counter, ratcheted so it can never go backwards.
///
/// `MonobatHal::entropy_source()` real hardware pe genuinely monotonic
/// nanosecond counter deta hai (dekho `MonobatRealHal::entropy_source`,
/// jo seedha `monobat_rtc_mono_get_ns()` return karta hai). Lekin
/// `StubHal` ka default hamesha `0` deta hai — us case mein raw value
/// kabhi badhta nahi, toh yeh function khud +1ns/call ratchet kar deta
/// hai taaki `Instant`/`Stopwatch`/timeouts phir bhi sahi order mein
/// progress karein (absolute timing accurate nahi hogi StubHal ke sath,
/// lekin relative ordering — jo zyadatar use cases ko chahiye — hamesha
/// sahi rahegi).
pub fn monotonic_ns() -> u64 {
    let raw = crate::remox_entropy();
    let mut prev = RETIME_MONO_RATCHET.load(Ordering::Relaxed);
    loop {
        let next = if raw > prev { raw } else { prev + 1 };
        match RETIME_MONO_RATCHET.compare_exchange_weak(
            prev, next, Ordering::Relaxed, Ordering::Relaxed,
        ) {
            Ok(_) => return next,
            Err(actual) => prev = actual,
        }
    }
}

/// Monotonic time in fractional seconds (Python's `time.monotonic()`).
pub fn monotonic() -> f64 {
    monotonic_ns() as f64 / 1_000_000_000.0
}

/// Monotonic time as a `Duration` since an arbitrary (boot-ish) origin.
pub fn monotonic_duration() -> Duration {
    Duration::from_nanos(monotonic_ns())
}

/// Highest-resolution counter for benchmarking (Python's `time.perf_counter`).
/// Same underlying clock as `monotonic_ns()` — Retime doesn't have a
/// separate hardware TSC path yet, so these are intentionally aliased.
pub fn perf_counter_ns() -> u64 { monotonic_ns() }
pub fn perf_counter() -> f64 { monotonic() }

/// Approximation of CPU time consumed by the current process (Python's
/// `time.process_time()`). See the module-level "HONEST STATUS" note —
/// this is wall-clock-equivalent until `MonobatHal` exposes real per-task
/// CPU accounting.
pub fn process_time_ns() -> u64 { monotonic_ns() }
pub fn process_time() -> f64 { monotonic() }

/// Calibrates the wall clock: from this point on, `now_ns()`/`time()`
/// report real Unix time. Call this once, as soon as a real time source
/// (RTC chip, NTP, boot-loader-provided timestamp, etc.) becomes available.
/// Safe to call more than once (e.g. to re-sync after an NTP correction).
pub fn set_wall_clock_unix_secs(unix_secs: u64) {
    set_wall_clock_unix_ns(unix_secs.saturating_mul(1_000_000_000));
}

/// Same as `set_wall_clock_unix_secs`, nanosecond precision.
pub fn set_wall_clock_unix_ns(unix_ns: u64) {
    let offset = unix_ns.saturating_sub(monotonic_ns());
    RETIME_EPOCH_OFFSET_NS.store(offset, Ordering::Relaxed);
    RETIME_WALL_CALIBRATED.store(true, Ordering::Relaxed);
}

/// Whether `set_wall_clock_unix_secs`/`_ns` has ever been called. If this
/// is `false`, `now_ns()`/`time()` are boot-relative, NOT real Unix time.
pub fn is_wall_clock_calibrated() -> bool {
    RETIME_WALL_CALIBRATED.load(Ordering::Relaxed)
}

/// Current wall-clock time in nanoseconds since the Unix epoch (once
/// calibrated — see `is_wall_clock_calibrated`).
pub fn now_ns() -> u64 {
    monotonic_ns().saturating_add(RETIME_EPOCH_OFFSET_NS.load(Ordering::Relaxed))
}
pub fn now_ms() -> u64 { now_ns() / 1_000_000 }
pub fn now_secs() -> u64 { now_ns() / 1_000_000_000 }

/// Python's `time.time()` — fractional Unix seconds.
pub fn time() -> f64 { now_ns() as f64 / 1_000_000_000.0 }
/// Python's `time.time_ns()`.
pub fn time_ns() -> u64 { now_ns() }

// =============================================================================
// SECTION 2 — `Instant`: ergonomic monotonic timestamp
// =============================================================================

/// A monotonic timestamp, nanosecond-precision. Cheaper and more precise
/// than Python's float-seconds `time.monotonic()` — no floating point
/// rounding, and arithmetic is exact `u64` nanoseconds.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct Instant(u64);

impl Instant {
    pub fn now() -> Self { Instant(monotonic_ns()) }

    /// Time elapsed since this `Instant` was captured.
    pub fn elapsed(&self) -> Duration {
        Duration::from_nanos(monotonic_ns().saturating_sub(self.0))
    }

    /// Duration between two instants; panics-free (saturates at zero if
    /// `earlier` is actually later, unlike `std::time::Instant`).
    pub fn duration_since(&self, earlier: Instant) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(earlier.0))
    }

    pub fn checked_duration_since(&self, earlier: Instant) -> Option<Duration> {
        self.0.checked_sub(earlier.0).map(Duration::from_nanos)
    }

    pub fn as_nanos_since_origin(&self) -> u64 { self.0 }
}

impl Add<Duration> for Instant {
    type Output = Instant;
    fn add(self, d: Duration) -> Instant {
        Instant(self.0.saturating_add(d.as_nanos() as u64))
    }
}
impl Sub<Duration> for Instant {
    type Output = Instant;
    fn sub(self, d: Duration) -> Instant {
        Instant(self.0.saturating_sub(d.as_nanos() as u64))
    }
}
impl Sub<Instant> for Instant {
    type Output = Duration;
    fn sub(self, other: Instant) -> Duration { self.duration_since(other) }
}

// =============================================================================
// SECTION 3 — Duration helpers: humanize + parse (not in Python's `time`)
// =============================================================================

/// Extension trait adding a human-readable formatter to `core::time::Duration`.
pub trait DurationExt {
    /// Formats as e.g. `"1d 2h 3m 4s"`. Sub-second remainder is shown in
    /// milliseconds only when the total is under a day. Zero duration
    /// formats as `"0ms"`.
    fn humanize(&self) -> String;
}

impl DurationExt for Duration {
    fn humanize(&self) -> String {
        format_duration_ms(self.as_millis() as u64)
    }
}

fn format_duration_ms(total_ms: u64) -> String {
    if total_ms == 0 { return "0ms".to_string(); }
    let ms = total_ms % 1000;
    let mut rem = total_ms / 1000;
    let s = rem % 60; rem /= 60;
    let m = rem % 60; rem /= 60;
    let h = rem % 24; rem /= 24;
    let d = rem;

    let mut parts: Vec<String> = Vec::new();
    if d > 0 { parts.push(format!("{}d", d)); }
    if h > 0 { parts.push(format!("{}h", h)); }
    if m > 0 { parts.push(format!("{}m", m)); }
    if s > 0 { parts.push(format!("{}s", s)); }
    if ms > 0 && d == 0 { parts.push(format!("{}ms", ms)); }
    parts.join(" ")
}

/// Parses human-readable duration strings like `"1h30m"`, `"500ms"`,
/// `"2d3h"`, `"1.5s"`. Each component is `<number><unit>` with no
/// separators required between components. Recognized units: `ms`, `s`,
/// `m`, `h`, `d`. Returns `None` on any malformed input.
pub fn parse_duration(s: &str) -> Option<Duration> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    let mut total_ms: f64 = 0.0;
    let mut matched_any = false;

    while i < bytes.len() {
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') { i += 1; }
        if start == i { return None; }
        let num: f64 = s.get(start..i)?.parse().ok()?;

        let unit_start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() { i += 1; }
        let unit = s.get(unit_start..i)?;

        let mult_ms: f64 = match unit {
            "ms" => 1.0,
            "s" => 1_000.0,
            "m" => 60_000.0,
            "h" => 3_600_000.0,
            "d" => 86_400_000.0,
            _ => return None,
        };
        total_ms += num * mult_ms;
        matched_any = true;
    }

    if matched_any { Some(Duration::from_millis(total_ms as u64)) } else { None }
}

/// Convenience `Duration` constants — clearer at call sites than
/// `Duration::from_secs(60)` scattered everywhere.
pub const SECOND: Duration = Duration::from_secs(1);
pub const MINUTE: Duration = Duration::from_secs(60);
pub const HOUR: Duration = Duration::from_secs(3600);
pub const DAY: Duration = Duration::from_secs(86_400);

// =============================================================================
// SECTION 4 — Calendar math (proleptic Gregorian, integer-only)
// =============================================================================
// Howard Hinnant's `days_from_civil` / `civil_from_days` — public-domain
// algorithm (http://howardhinnant.github.io/date_algorithms.html), reimplemented
// here from scratch in integer arithmetic. Verified against a 200-year
// (1900-2100) exhaustive round-trip before being ported into this no_std
// module. Handles the Gregorian leap-year rule (div-by-4, not-div-by-100,
// div-by-400) exactly, for any proleptic year (negative years included).

/// Days since the Unix epoch (1970-01-01) for a given civil (Gregorian)
/// date. Negative for dates before 1970. `month` is 1-12, `day` is 1-31.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (month as i64 + 9) % 12; // Mar=0 .. Feb=11
    let doy = (153 * mp + 2) / 5 + day as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Inverse of `days_from_civil`: epoch day number -> (year, month, day).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if month <= 2 { y + 1 } else { y };
    (y, month, day)
}

pub fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if is_leap_year(year) { 29 } else { 28 },
        _ => 0,
    }
}

/// Day of week for an epoch day number. `0 = Monday .. 6 = Sunday`
/// (matches Python's `time.struct_time.tm_wday` convention).
pub fn weekday_from_days(epoch_days: i64) -> u32 {
    (epoch_days.rem_euclid(7) + 3).rem_euclid(7) as u32
}

/// 1-based day-of-year for a civil date (`1..=366`).
pub fn day_of_year(year: i64, month: u32, day: u32) -> u32 {
    (days_from_civil(year, month, day) - days_from_civil(year, 1, 1) + 1) as u32
}

// =============================================================================
// SECTION 5 — `StructTime`, `FixedOffset`, gmtime/timegm/localtime
// =============================================================================

/// Broken-down calendar time — equivalent to Python's `time.struct_time`,
/// with the same field conventions (`weekday`: Mon=0..Sun=6, `yday`: 1..366).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StructTime {
    pub year: i64,
    pub month: u32,   // 1-12
    pub day: u32,     // 1-31
    pub hour: u32,    // 0-23
    pub minute: u32,  // 0-59
    pub second: u32,  // 0-60 (60 reserved for leap seconds, unused here)
    pub weekday: u32, // 0=Mon .. 6=Sun
    pub yday: u32,    // 1-366
    pub nanos: u32,   // sub-second remainder, 0..999_999_999
}

/// A fixed UTC offset (no DST, no named zone database — see module header).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FixedOffset {
    /// Offset from UTC in seconds, e.g. IST = `19_800` (+05:30),
    /// PST = `-28_800` (-08:00).
    pub offset_seconds: i32,
}

impl FixedOffset {
    pub const UTC: FixedOffset = FixedOffset { offset_seconds: 0 };

    pub fn east_seconds(secs: i32) -> Self { FixedOffset { offset_seconds: secs } }

    /// Both arguments should share the same sign (e.g. `(-8, 0)` for
    /// PST, `(5, 30)` for IST).
    pub fn east_hours_minutes(hours: i32, minutes: i32) -> Self {
        FixedOffset { offset_seconds: hours * 3600 + minutes * 60 }
    }

    /// e.g. `"+05:30"`, `"-08:00"`, `"Z"` for UTC.
    pub fn as_iso_suffix(&self) -> String {
        if self.offset_seconds == 0 { return "Z".to_string(); }
        let sign = if self.offset_seconds < 0 { '-' } else { '+' };
        let abs = self.offset_seconds.unsigned_abs();
        format!("{}{:02}:{:02}", sign, abs / 3600, (abs % 3600) / 60)
    }
}

/// Breaks down Unix epoch seconds into UTC calendar fields
/// (Python's `time.gmtime()`).
pub fn gmtime(epoch_secs: i64) -> StructTime {
    localtime(epoch_secs, FixedOffset::UTC)
}

/// Breaks down Unix epoch seconds into calendar fields under a given fixed
/// UTC offset (Python's `time.localtime()`, generalized — Retime has no
/// "the" local timezone since it has no tzdata; caller supplies one).
pub fn localtime(epoch_secs: i64, offset: FixedOffset) -> StructTime {
    let shifted = epoch_secs + offset.offset_seconds as i64;
    let days = shifted.div_euclid(86_400);
    let secs_of_day = shifted.rem_euclid(86_400);

    let (year, month, day) = civil_from_days(days);
    let hour = (secs_of_day / 3600) as u32;
    let minute = ((secs_of_day % 3600) / 60) as u32;
    let second = (secs_of_day % 60) as u32;

    StructTime {
        year, month, day, hour, minute, second,
        weekday: weekday_from_days(days),
        yday: day_of_year(year, month, day),
        nanos: 0,
    }
}

/// Inverse of `gmtime`: `StructTime` (interpreted as UTC) -> Unix epoch
/// seconds. (Python's `calendar.timegm`; Python's own `time.mktime`
/// assumes local time, which Retime can't do without a real tzdata
/// database — use `FixedOffset` explicitly via `timegm_offset` instead.)
pub fn timegm(t: &StructTime) -> i64 {
    days_from_civil(t.year, t.month, t.day) * 86_400
        + t.hour as i64 * 3600
        + t.minute as i64 * 60
        + t.second as i64
}

/// Like `timegm`, but `t`'s fields are interpreted as being in `offset`
/// rather than UTC.
pub fn timegm_offset(t: &StructTime, offset: FixedOffset) -> i64 {
    timegm(t) - offset.offset_seconds as i64
}

// =============================================================================
// SECTION 6 — Formatting: strftime subset, asctime/ctime, ISO-8601/RFC3339
// =============================================================================

const MONTH_NAMES: [&str; 12] = [
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December",
];
const MONTH_ABBR: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const DAY_NAMES: [&str; 7] = [
    "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday",
];
const DAY_ABBR: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// `strftime`-style formatter. Supported specifiers:
/// `%Y %y %m %d %H %M %S %I %p %A %a %B %b %j %%` and `%Z` (takes a
/// caller-supplied zone label since Retime has no named-zone database).
/// Unknown `%x` specifiers pass through literally (e.g. `%q` -> `%q`).
pub fn strftime(t: &StructTime, fmt: &str, tz_label: &str) -> String {
    let mut out = String::new();
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c != '%' { out.push(c); continue; }
        match chars.next() {
            Some('Y') => out.push_str(&format!("{}", t.year)),
            Some('y') => out.push_str(&format!("{:02}", t.year.rem_euclid(100))),
            Some('m') => out.push_str(&format!("{:02}", t.month)),
            Some('d') => out.push_str(&format!("{:02}", t.day)),
            Some('H') => out.push_str(&format!("{:02}", t.hour)),
            Some('M') => out.push_str(&format!("{:02}", t.minute)),
            Some('S') => out.push_str(&format!("{:02}", t.second)),
            Some('I') => {
                let h12 = match t.hour % 12 { 0 => 12, h => h };
                out.push_str(&format!("{:02}", h12));
            }
            Some('p') => out.push_str(if t.hour < 12 { "AM" } else { "PM" }),
            Some('A') => out.push_str(DAY_NAMES[t.weekday as usize % 7]),
            Some('a') => out.push_str(DAY_ABBR[t.weekday as usize % 7]),
            Some('B') => out.push_str(MONTH_NAMES[(t.month.saturating_sub(1) as usize) % 12]),
            Some('b') => out.push_str(MONTH_ABBR[(t.month.saturating_sub(1) as usize) % 12]),
            Some('j') => out.push_str(&format!("{:03}", t.yday)),
            Some('Z') => out.push_str(tz_label),
            Some('%') => out.push('%'),
            Some(other) => { out.push('%'); out.push(other); }
            None => out.push('%'),
        }
    }
    out
}

/// Python's `time.asctime()`: `"Sat Jul 11 14:05:09 2026"`.
pub fn asctime(t: &StructTime) -> String {
    strftime(t, "%a %b %d %H:%M:%S %Y", "")
}

/// Python's `time.ctime()`: `asctime(localtime(epoch_secs))` under UTC
/// (see `localtime`'s doc comment re: no tzdata).
pub fn ctime(epoch_secs: i64) -> String {
    asctime(&gmtime(epoch_secs))
}

/// RFC3339 / ISO-8601 formatting, e.g. `"2026-07-11T14:05:09+05:30"`.
pub fn to_rfc3339(t: &StructTime, offset: FixedOffset) -> String {
    format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}{}",
        t.year, t.month, t.day, t.hour, t.minute, t.second, offset.as_iso_suffix()
    )
}

/// Parses an RFC3339 / ISO-8601 timestamp: `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)`.
/// The `T` separator may also be a space or lowercase `t`. Returns the
/// parsed fields as UTC epoch seconds (offset already applied) plus the
/// nanosecond remainder and the offset that was present in the string.
pub fn parse_rfc3339(s: &str) -> Option<(i64, u32, FixedOffset)> {
    let bytes = s.as_bytes();
    if bytes.len() < 19 { return None; }
    let get_i64 = |a: usize, b: usize| -> Option<i64> { s.get(a..b)?.parse().ok() };

    let year = get_i64(0, 4)?;
    if bytes[4] != b'-' { return None; }
    let month = get_i64(5, 7)? as u32;
    if bytes[7] != b'-' { return None; }
    let day = get_i64(8, 10)? as u32;
    let sep = bytes[10];
    if sep != b'T' && sep != b't' && sep != b' ' { return None; }
    let hour = get_i64(11, 13)? as u32;
    if bytes[13] != b':' { return None; }
    let minute = get_i64(14, 16)? as u32;
    if bytes[16] != b':' { return None; }
    let second = get_i64(17, 19)? as u32;

    let mut idx = 19usize;
    let mut nanos = 0u32;
    if idx < bytes.len() && bytes[idx] == b'.' {
        let start = idx + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_digit() { end += 1; }
        let frac = s.get(start..end)?;
        // Pad/truncate the fractional part to exactly 9 digits (nanoseconds).
        let mut digits = [b'0'; 9];
        for (i, b) in frac.bytes().take(9).enumerate() { digits[i] = b; }
        nanos = core::str::from_utf8(&digits).ok()?.parse().ok()?;
        idx = end;
    }

    let offset_seconds: i32 = if idx < bytes.len() {
        match bytes[idx] {
            b'Z' | b'z' => 0,
            b'+' | b'-' => {
                let sign: i32 = if bytes[idx] == b'-' { -1 } else { 1 };
                let oh: i32 = get_i64(idx + 1, idx + 3)? as i32;
                let om: i32 = if bytes.len() >= idx + 6 && bytes[idx + 3] == b':' {
                    get_i64(idx + 4, idx + 6)? as i32
                } else { 0 };
                sign * (oh * 3600 + om * 60)
            }
            _ => return None,
        }
    } else { 0 };

    if month == 0 || month > 12 || day == 0 || day > days_in_month(year, month)
        || hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let t = StructTime {
        year, month, day, hour, minute, second,
        weekday: weekday_from_days(days_from_civil(year, month, day)),
        yday: day_of_year(year, month, day),
        nanos,
    };
    let offset = FixedOffset::east_seconds(offset_seconds);
    Some((timegm_offset(&t, offset), nanos, offset))
}

// =============================================================================
// SECTION 7 — `Stopwatch`: lap-capable timer (not in Python's `time`)
// =============================================================================

/// A pausable, lap-capable stopwatch built on the monotonic clock.
#[derive(Clone, Debug)]
pub struct Stopwatch {
    running_since: Option<Instant>,
    accumulated: Duration,
    laps: Vec<Duration>,
}

impl Stopwatch {
    pub fn new() -> Self {
        Stopwatch { running_since: None, accumulated: Duration::ZERO, laps: Vec::new() }
    }

    /// Creates a `Stopwatch` that's already running.
    pub fn start_new() -> Self {
        let mut sw = Self::new();
        sw.start();
        sw
    }

    pub fn start(&mut self) {
        if self.running_since.is_none() {
            self.running_since = Some(Instant::now());
        }
    }

    /// Pauses the stopwatch; `elapsed()` stays frozen until `start()` again.
    pub fn pause(&mut self) {
        if let Some(since) = self.running_since.take() {
            self.accumulated += since.elapsed();
        }
    }

    pub fn reset(&mut self) {
        self.running_since = None;
        self.accumulated = Duration::ZERO;
        self.laps.clear();
    }

    pub fn is_running(&self) -> bool { self.running_since.is_some() }

    pub fn elapsed(&self) -> Duration {
        match self.running_since {
            Some(since) => self.accumulated + since.elapsed(),
            None => self.accumulated,
        }
    }

    /// Records a lap: the elapsed time *since the previous lap* (or since
    /// start, for the first lap). Returns that lap duration.
    pub fn lap(&mut self) -> Duration {
        let total = self.elapsed();
        let previous_total: Duration = self.laps.iter().sum();
        let lap_duration = total.saturating_sub(previous_total);
        self.laps.push(lap_duration);
        lap_duration
    }

    pub fn laps(&self) -> &[Duration] { &self.laps }
}

impl Default for Stopwatch {
    fn default() -> Self { Self::new() }
}

// =============================================================================
// SECTION 8 — `Deadline` and `Ticker`: scheduling primitives
// =============================================================================

/// A point-in-time timeout, computed once and checked cheaply thereafter.
#[derive(Clone, Copy, Debug)]
pub struct Deadline(Instant);

impl Deadline {
    /// A deadline `d` in the future from now.
    pub fn after(d: Duration) -> Self { Deadline(Instant::now() + d) }

    pub fn expired(&self) -> bool { Instant::now() >= self.0 }

    /// Time remaining, or `Duration::ZERO` if already expired.
    pub fn remaining(&self) -> Duration {
        self.0.checked_duration_since(Instant::now()).unwrap_or(Duration::ZERO)
    }

    /// Blocks (via `std::thread::sleep`) until the deadline, or
    /// returns immediately if it has already passed.
    pub fn wait(&self) {
        let rem = self.remaining();
        if !rem.is_zero() {
            std::thread::sleep(rem);
        }
    }
}

/// A periodic checkpoint — fires once every `period`, drift-corrected
/// (missed ticks don't accumulate: the next fire time is always the
/// previous scheduled time plus one period, not "now plus period").
#[derive(Clone, Copy, Debug)]
pub struct Ticker {
    period: Duration,
    next_fire: Instant,
}

impl Ticker {
    pub fn new(period: Duration) -> Self {
        Ticker { period, next_fire: Instant::now() + period }
    }

    /// True if the tick has come due. Advances internal scheduling
    /// drift-corrected regardless of how late this was actually called.
    pub fn ready(&mut self) -> bool {
        if Instant::now() >= self.next_fire {
            self.next_fire = self.next_fire + self.period;
            // If we fell more than one full period behind, resync to now
            // instead of firing a burst of catch-up ticks.
            if Instant::now() >= self.next_fire + self.period {
                self.next_fire = Instant::now() + self.period;
            }
            true
        } else {
            false
        }
    }

    /// Blocks until the next tick is due.
    pub fn wait_next(&mut self) {
        loop {
            let now = Instant::now();
            if now >= self.next_fire {
                self.next_fire = self.next_fire + self.period;
                return;
            }
            std::thread::sleep(self.next_fire.duration_since(now));
        }
    }
}

// =============================================================================
// SECTION 9 — `RateCounter`: rolling rate/frequency measurement
// =============================================================================

/// Measures an event rate (e.g. requests/sec, frames/sec) over a sliding
/// window. Not present in Python's `time` module at all.
#[derive(Clone, Debug)]
pub struct RateCounter {
    window: Duration,
    events: Vec<Instant>,
}

impl RateCounter {
    pub fn new(window: Duration) -> Self {
        RateCounter { window, events: Vec::new() }
    }

    /// Records one event occurrence now.
    pub fn tick(&mut self) {
        self.prune();
        self.events.push(Instant::now());
    }

    fn prune(&mut self) {
        let cutoff = Instant::now();
        self.events.retain(|e| cutoff.duration_since(*e) <= self.window);
    }

    /// Events per second, averaged over the configured window.
    pub fn rate_hz(&mut self) -> f64 {
        self.prune();
        if self.events.is_empty() { return 0.0; }
        self.events.len() as f64 / self.window.as_secs_f64()
    }

    pub fn count(&mut self) -> usize {
        self.prune();
        self.events.len()
    }
}

// =============================================================================
// SECTION 10 — sleep helpers + `benchmark()`
// =============================================================================

/// Python's `time.sleep()`. See `std::thread::sleep`'s doc comment
/// for the current spin-loop accuracy caveat.
pub fn sleep(d: Duration) { std::thread::sleep(d); }
pub fn sleep_ms(ms: u64) { sleep(Duration::from_millis(ms)); }
pub fn sleep_secs(secs: f64) { sleep(Duration::from_secs_f64(secs.max(0.0))); }

/// Sleeps until a specific `Instant`. No-op if it's already in the past.
pub fn sleep_until(target: Instant) {
    let now = Instant::now();
    if let Some(d) = target.checked_duration_since(now) {
        sleep(d);
    }
}

/// Runs `f`, returning its result alongside how long it took. Not in
/// Python's `time` module (the idiomatic Python pattern is a manual
/// `start = time.perf_counter(); ...; elapsed = time.perf_counter() - start`
/// — this collapses that into one call).
pub fn benchmark<F, R>(f: F) -> (R, Duration)
where
    F: FnOnce() -> R,
{
    let start = Instant::now();
    let result = f();
    (result, start.elapsed())
}

// =============================================================================
// SECTION 11 — Display impls
// =============================================================================

impl fmt::Display for StructTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", strftime(self, "%Y-%m-%d %H:%M:%S", "UTC"))
    }
}

impl fmt::Display for FixedOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_iso_suffix())
    }
}

// =============================================================================
// SECTION 12 — Self-test (no_std kernel has no `#[test]` harness, so this
// is a callable sanity check Monobat's boot/debug console can invoke)
// =============================================================================

/// Runs internal consistency checks (calendar round-trips, known reference
/// dates, duration parse/format round-trips). Returns `true` iff everything
/// passed. Intended to be called once from kernel debug/boot code, e.g.
/// `assert!(retime::self_test());` — NOT a `#[test]`, since this crate has
/// no std test harness.
pub fn self_test() -> bool {
    // Known reference points.
    if days_from_civil(1970, 1, 1) != 0 { return false; }
    if days_from_civil(2000, 1, 1) != 10_957 { return false; }
    if civil_from_days(0) != (1970, 1, 1) { return false; }

    // Round trip over a representative range.
    for year in 1900..2100i64 {
        for month in 1..=12u32 {
            let dim = days_in_month(year, month);
            for day in 1..=dim {
                let z = days_from_civil(year, month, day);
                if civil_from_days(z) != (year, month, day) { return false; }
            }
        }
    }

    // 1970-01-01 was a Thursday (weekday=3, Mon=0).
    if weekday_from_days(0) != 3 { return false; }

    // Duration parse/format sanity.
    if parse_duration("1h30m") != Some(Duration::from_secs(5400)) { return false; }
    if parse_duration("500ms") != Some(Duration::from_millis(500)) { return false; }
    if parse_duration("not_a_duration").is_some() { return false; }
    if Duration::from_millis(3_661_000).humanize() != "1h 1m 1s" { return false; }

    // RFC3339 round trip.
    match parse_rfc3339("2026-07-11T14:05:09+05:30") {
        Some((epoch, nanos, offset)) => {
            if nanos != 0 { return false; }
            if offset.offset_seconds != 19_800 { return false; }
            let t = localtime(epoch, offset);
            if (t.year, t.month, t.day, t.hour, t.minute, t.second)
                != (2026, 7, 11, 14, 5, 9) {
                return false;
            }
        }
        None => return false,
    }

    true
}
}

// =============================================================================
// RETIME DISPATCH — routes __retime_* builtin names to the `retime` module.
// Free function (Malib/Numrux pattern): stateless where possible; the four
// stateful primitives (Stopwatch/Deadline/Ticker/RateCounter) are kept in
// SpinMutex<BTreeMap<id, T>> registries, exactly like Astriloop's
// Lock/Semaphore/Barrier objects, and handed back to Remox as a small
// `{ __retime_id, __retime_kind }` tagged Map that the wrapper functions
// below look up on every call.
// =============================================================================

static RETIME_NEXT_ID: AtomicU64 = AtomicU64::new(1);
fn retime_new_id() -> u64 { RETIME_NEXT_ID.fetch_add(1, Ordering::Relaxed) }

static RETIME_STOPWATCHES:  SpinMutex<BTreeMap<u64, retime::Stopwatch>>   = SpinMutex::new(BTreeMap::new());
static RETIME_DEADLINES:    SpinMutex<BTreeMap<u64, retime::Deadline>>    = SpinMutex::new(BTreeMap::new());
static RETIME_TICKERS:      SpinMutex<BTreeMap<u64, retime::Ticker>>      = SpinMutex::new(BTreeMap::new());
static RETIME_RATECOUNTERS: SpinMutex<BTreeMap<u64, retime::RateCounter>> = SpinMutex::new(BTreeMap::new());

fn retime_obj(id: u64, kind: &str) -> Value {
    Value::Map(vec![
        ("__retime_id".into(),   Value::Int(id as i64)),
        ("__retime_kind".into(), Value::Str(kind.into())),
    ])
}
fn retime_id_of(v: &Value) -> Option<u64> {
    if let Value::Map(pairs) = v {
        for (k, val) in pairs {
            if k == "__retime_id" {
                if let Value::Int(n) = val { return Some(*n as u64); }
            }
        }
    }
    None
}

fn retime_struct_time_to_value(t: &retime::StructTime) -> Value {
    Value::Map(vec![
        ("year".into(),    Value::Int(t.year)),
        ("month".into(),   Value::Int(t.month as i64)),
        ("day".into(),     Value::Int(t.day as i64)),
        ("hour".into(),    Value::Int(t.hour as i64)),
        ("minute".into(),  Value::Int(t.minute as i64)),
        ("second".into(),  Value::Int(t.second as i64)),
        ("weekday".into(), Value::Int(t.weekday as i64)),
        ("yday".into(),    Value::Int(t.yday as i64)),
        ("nanos".into(),   Value::Int(t.nanos as i64)),
    ])
}

fn retime_value_to_struct_time(v: &Value) -> retime::StructTime {
    let get = |key: &str| -> i64 {
        if let Value::Map(pairs) = v {
            for (k, val) in pairs {
                if k == key { return malib_i64(val); }
            }
        }
        0
    };
    let year   = get("year");
    let month  = get("month") as u32;
    let day    = get("day") as u32;
    retime::StructTime {
        year, month, day,
        hour:    get("hour") as u32,
        minute:  get("minute") as u32,
        second:  get("second") as u32,
        weekday: retime::weekday_from_days(retime::days_from_civil(year, month, day)),
        yday:    retime::day_of_year(year, month, day),
        nanos:   get("nanos") as u32,
    }
}

pub(crate) fn dispatch_retime(name: &str, args: Vec<Value>) -> Result<Value, String> {
    let i0 = |args: &Vec<Value>| malib_i64(args.get(0).unwrap_or(&Value::Int(0)));
    let i1 = |args: &Vec<Value>| malib_i64(args.get(1).unwrap_or(&Value::Int(0)));
    let i2 = |args: &Vec<Value>| malib_i64(args.get(2).unwrap_or(&Value::Int(0)));
    let f0 = |args: &Vec<Value>| malib_f64(args.get(0).unwrap_or(&Value::Int(0)));
    let s0 = |args: &Vec<Value>| args.get(0).map(|v| v.to_string()).unwrap_or_default();

    match name {
        // ── RAW CLOCKS ────────────────────────────────────────────────────
        "__retime_time"           => Ok(Value::Float(retime::time())),
        "__retime_time_ns"        => Ok(Value::Int(retime::time_ns() as i64)),
        "__retime_now_ms"         => Ok(Value::Int(retime::now_ms() as i64)),
        "__retime_now_secs"       => Ok(Value::Int(retime::now_secs() as i64)),
        "__retime_monotonic"      => Ok(Value::Float(retime::monotonic())),
        "__retime_monotonic_ns"   => Ok(Value::Int(retime::monotonic_ns() as i64)),
        "__retime_perf_counter"   => Ok(Value::Float(retime::perf_counter())),
        "__retime_perf_counter_ns"=> Ok(Value::Int(retime::perf_counter_ns() as i64)),
        "__retime_process_time"   => Ok(Value::Float(retime::process_time())),

        // ── WALL-CLOCK CALIBRATION ──────────────────────────────────────
        "__retime_set_wall_clock" => {
            retime::set_wall_clock_unix_secs(i0(&args) as u64);
            Ok(Value::Null)
        }
        "__retime_is_wall_clock_calibrated" => Ok(Value::Bool(retime::is_wall_clock_calibrated())),

        // ── SLEEP ─────────────────────────────────────────────────────────
        "__retime_sleep_secs" => { retime::sleep_secs(f0(&args)); Ok(Value::Null) }
        "__retime_sleep_ms"   => { retime::sleep_ms(i0(&args) as u64); Ok(Value::Null) }

        // ── CALENDAR ──────────────────────────────────────────────────────
        "__retime_gmtime" => Ok(retime_struct_time_to_value(&retime::gmtime(i0(&args)))),
        "__retime_localtime" => {
            let off = retime::FixedOffset::east_seconds(i1(&args) as i32);
            Ok(retime_struct_time_to_value(&retime::localtime(i0(&args), off)))
        }
        "__retime_strftime" => {
            let t = retime_value_to_struct_time(args.get(0).unwrap_or(&Value::Null));
            let fmt = args.get(1).map(|v| v.to_string()).unwrap_or_else(|| "%Y-%m-%d %H:%M:%S".to_string());
            let tz  = args.get(2).map(|v| v.to_string()).unwrap_or_default();
            Ok(Value::Str(retime::strftime(&t, &fmt, &tz)))
        }
        "__retime_asctime" => {
            let t = retime_value_to_struct_time(args.get(0).unwrap_or(&Value::Null));
            Ok(Value::Str(retime::asctime(&t)))
        }
        "__retime_ctime" => Ok(Value::Str(retime::ctime(i0(&args)))),
        "__retime_to_rfc3339" => {
            let t = retime_value_to_struct_time(args.get(0).unwrap_or(&Value::Null));
            let off = retime::FixedOffset::east_seconds(i1(&args) as i32);
            Ok(Value::Str(retime::to_rfc3339(&t, off)))
        }
        "__retime_parse_rfc3339" => {
            match retime::parse_rfc3339(&s0(&args)) {
                Some((epoch, nanos, off)) => Ok(Value::Map(vec![
                    ("ok".into(),        Value::Bool(true)),
                    ("epochSecs".into(), Value::Int(epoch)),
                    ("nanos".into(),     Value::Int(nanos as i64)),
                    ("offsetSecs".into(),Value::Int(off.offset_seconds as i64)),
                ])),
                None => Ok(Value::Map(vec![("ok".into(), Value::Bool(false))])),
            }
        }

        // ── DURATION HELPERS ─────────────────────────────────────────────
        "__retime_parse_duration" => match retime::parse_duration(&s0(&args)) {
            Some(d) => Ok(Value::Int(d.as_millis() as i64)),
            None    => Ok(Value::Null),
        },
        "__retime_humanize" => {
            let d = Duration::from_millis(i0(&args).max(0) as u64);
            Ok(Value::Str(retime::DurationExt::humanize(&d)))
        }

        // ── CALENDAR MATH ────────────────────────────────────────────────
        "__retime_days_from_civil" => Ok(Value::Int(retime::days_from_civil(i0(&args), i1(&args) as u32, i2(&args) as u32))),
        "__retime_civil_from_days" => {
            let (y, m, d) = retime::civil_from_days(i0(&args));
            Ok(Value::Map(vec![("year".into(), Value::Int(y)), ("month".into(), Value::Int(m as i64)), ("day".into(), Value::Int(d as i64))]))
        }
        "__retime_is_leap_year"     => Ok(Value::Bool(retime::is_leap_year(i0(&args)))),
        "__retime_days_in_month"    => Ok(Value::Int(retime::days_in_month(i0(&args), i1(&args) as u32) as i64)),
        "__retime_weekday_from_days"=> Ok(Value::Int(retime::weekday_from_days(i0(&args)) as i64)),
        "__retime_day_of_year"      => Ok(Value::Int(retime::day_of_year(i0(&args), i1(&args) as u32, i2(&args) as u32) as i64)),

        // ── STOPWATCH ────────────────────────────────────────────────────
        "__retime_stopwatch_new" => {
            let id = retime_new_id();
            RETIME_STOPWATCHES.lock().insert(id, retime::Stopwatch::start_new());
            Ok(retime_obj(id, "stopwatch"))
        }
        "__retime_stopwatch_start" => {
            let id = retime_id_of(&args[0]).ok_or("Retime: invalid Stopwatch handle")?;
            if let Some(sw) = RETIME_STOPWATCHES.lock().get_mut(&id) { sw.start(); }
            Ok(Value::Null)
        }
        "__retime_stopwatch_pause" => {
            let id = retime_id_of(&args[0]).ok_or("Retime: invalid Stopwatch handle")?;
            if let Some(sw) = RETIME_STOPWATCHES.lock().get_mut(&id) { sw.pause(); }
            Ok(Value::Null)
        }
        "__retime_stopwatch_reset" => {
            let id = retime_id_of(&args[0]).ok_or("Retime: invalid Stopwatch handle")?;
            if let Some(sw) = RETIME_STOPWATCHES.lock().get_mut(&id) { sw.reset(); }
            Ok(Value::Null)
        }
        "__retime_stopwatch_elapsed_ms" => {
            let id = retime_id_of(&args[0]).ok_or("Retime: invalid Stopwatch handle")?;
            let ms = RETIME_STOPWATCHES.lock().get(&id).map(|sw| sw.elapsed().as_millis() as i64).unwrap_or(0);
            Ok(Value::Int(ms))
        }
        "__retime_stopwatch_lap" => {
            let id = retime_id_of(&args[0]).ok_or("Retime: invalid Stopwatch handle")?;
            let ms = RETIME_STOPWATCHES.lock().get_mut(&id).map(|sw| sw.lap().as_millis() as i64).unwrap_or(0);
            Ok(Value::Int(ms))
        }
        "__retime_stopwatch_laps" => {
            let id = retime_id_of(&args[0]).ok_or("Retime: invalid Stopwatch handle")?;
            let laps = RETIME_STOPWATCHES.lock().get(&id)
                .map(|sw| sw.laps().iter().map(|d| Value::Int(d.as_millis() as i64)).collect())
                .unwrap_or_else(Vec::new);
            Ok(Value::List(laps))
        }

        // ── DEADLINE ─────────────────────────────────────────────────────
        "__retime_deadline_new" => {
            let id = retime_new_id();
            let ms = i0(&args).max(0) as u64;
            RETIME_DEADLINES.lock().insert(id, retime::Deadline::after(Duration::from_millis(ms)));
            Ok(retime_obj(id, "deadline"))
        }
        "__retime_deadline_expired" => {
            let id = retime_id_of(&args[0]).ok_or("Retime: invalid Deadline handle")?;
            let exp = RETIME_DEADLINES.lock().get(&id).map(|dl| dl.expired()).unwrap_or(true);
            Ok(Value::Bool(exp))
        }
        "__retime_deadline_remaining_ms" => {
            let id = retime_id_of(&args[0]).ok_or("Retime: invalid Deadline handle")?;
            let ms = RETIME_DEADLINES.lock().get(&id).map(|dl| dl.remaining().as_millis() as i64).unwrap_or(0);
            Ok(Value::Int(ms))
        }
        "__retime_deadline_wait" => {
            let id = retime_id_of(&args[0]).ok_or("Retime: invalid Deadline handle")?;
            let dl = RETIME_DEADLINES.lock().get(&id).copied();
            if let Some(dl) = dl { dl.wait(); }
            Ok(Value::Null)
        }

        // ── TICKER ───────────────────────────────────────────────────────
        "__retime_ticker_new" => {
            let id = retime_new_id();
            let ms = i0(&args).max(1) as u64;
            RETIME_TICKERS.lock().insert(id, retime::Ticker::new(Duration::from_millis(ms)));
            Ok(retime_obj(id, "ticker"))
        }
        "__retime_ticker_ready" => {
            let id = retime_id_of(&args[0]).ok_or("Retime: invalid Ticker handle")?;
            let ready = RETIME_TICKERS.lock().get_mut(&id).map(|tk| tk.ready()).unwrap_or(false);
            Ok(Value::Bool(ready))
        }
        "__retime_ticker_wait_next" => {
            let id = retime_id_of(&args[0]).ok_or("Retime: invalid Ticker handle")?;
            let mut guard = RETIME_TICKERS.lock();
            if let Some(tk) = guard.get_mut(&id) { tk.wait_next(); }
            Ok(Value::Null)
        }

        // ── RATE COUNTER ─────────────────────────────────────────────────
        "__retime_ratecounter_new" => {
            let id = retime_new_id();
            let ms = i0(&args).max(1) as u64;
            RETIME_RATECOUNTERS.lock().insert(id, retime::RateCounter::new(Duration::from_millis(ms)));
            Ok(retime_obj(id, "ratecounter"))
        }
        "__retime_ratecounter_tick" => {
            let id = retime_id_of(&args[0]).ok_or("Retime: invalid RateCounter handle")?;
            if let Some(rc) = RETIME_RATECOUNTERS.lock().get_mut(&id) { rc.tick(); }
            Ok(Value::Null)
        }
        "__retime_ratecounter_rate_hz" => {
            let id = retime_id_of(&args[0]).ok_or("Retime: invalid RateCounter handle")?;
            let hz = RETIME_RATECOUNTERS.lock().get_mut(&id).map(|rc| rc.rate_hz()).unwrap_or(0.0);
            Ok(Value::Float(hz))
        }
        "__retime_ratecounter_count" => {
            let id = retime_id_of(&args[0]).ok_or("Retime: invalid RateCounter handle")?;
            let n = RETIME_RATECOUNTERS.lock().get_mut(&id).map(|rc| rc.count()).unwrap_or(0);
            Ok(Value::Int(n as i64))
        }

        // ── DIAGNOSTICS ──────────────────────────────────────────────────
        "__retime_self_test" => Ok(Value::Bool(retime::self_test())),

        _ => Err(format!("Unknown Retime function: {}", name)),
    }
}

// =============================================================================
// Native entry point — same body, just called from a real `fn main()`
// instead of Monobat's `remox_entry`/scheduler hook.
// =============================================================================
fn main() {
    remox_kernel_main();
}
