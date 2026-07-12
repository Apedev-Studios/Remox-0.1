# Remox 🚀
**Easier than Python. Faster than Kotlin.**

Remox is a compiled scripting language written in Rust — designed to be the simplest language you've ever used, without sacrificing speed. One file. Zero dependencies beyond a Rust toolchain.

---

## Quick Start

```bash
# Clone and build
git clone https://github.com/Apedev-Studios/Remox-0.1
cd remox
cargo build --release

# Run a script
./target/release/remox examples/hello.remox

# REPL
./target/release/remox
```

---

## The Language

```remox
// Variables
let name = "Remox"
let version = 0.8
let ready = true

// String interpolation
say "Hello from {name} v{version}!"

// Functions
fn square(n) {
    n * n
}
say square(9)   // 81

// Lists
let nums = [3, 1, 4, 1, 5, 9]
say nums.sort()
say nums.sum()
say nums.unique()

// Maps
let user = { name: "Dev", age: 21 }
say user.name

// Pattern matching
match version {
    0.8 => say "Beta"
    1.0 => say "Stable"
    _   => say "Unknown"
}

// Loops
loop 5 {
    say "Counting..."
}

each item in nums {
    say item * 2
}

// Error handling
let result = try {
    risky_operation()
} catch err {
    say "Caught: {err}"
}
```

---

## Features

### Core Language
- `let`, `fn`, `if/else`, `loop`, `each`, `match`, `when`
- String interpolation `"Hello {name}"`
- Lambda / closures
- Structs, Traits, Impl blocks
- Pipe operator `|>`
- Pattern matching with destructuring
- Async/await (`use astriloop`)

### Built-in Libraries

| Library | Description |
|---|---|
| **Vyraweb** | HTTP server — routing, ORM, templates, WebSocket |
| **Remotest** | Testing framework — fixtures, mocks, property-based, load testing |
| **Autoclib** | CLI toolkit — styled output, prompts, TUI widgets, arg parsing |
| **Tasoaque** | Task queue — retries, rate limits, scheduling, cluster workers |
| **Astriloop** | Async runtime — events, channels, semaphores, stream pipelines |
| **Malib** | Math engine — algebra solver, calculus, matrices, number theory |
| **Phinolib** | Physics — fluid dynamics, special relativity, optics |
| **Numrux** | NumPy-equivalent — array ops, linear algebra, statistics |
| **Retime** | Time — calendar math, stopwatch, ticker, cron |
| **Remojoke** | Joke library — 100+ categories |

### Examples

```bash
cargo run --release -- examples/hello.remox
cargo run --release -- examples/fibonacci.remox
cargo run --release -- examples/web_server.remox
```

---

## Why Remox?

| | Python | Kotlin | Remox |
|---|---|---|---|
| Syntax simplicity | ✅ | ❌ | ✅ |
| Compiled speed | ❌ | ✅ | ✅ |
| Built-in web framework | ❌ | ❌ | ✅ |
| Built-in testing | ❌ | ❌ | ✅ |
| Built-in async | partial | partial | ✅ |
| Single file runtime | ❌ | ❌ | ✅ |
| Zero runtime deps | ❌ | ❌ | ✅ |

---

## Build Requirements

- Rust 1.75+ (`rustup` recommended)
- Any OS — Linux, macOS, Windows

---

## License

MIT — free to use, modify, distribute.

---

*Built by Aahil — part of the Apeion ecosystem.*
