# keen Syntax Specification

**Version 0.1 (draft) — source extension `.kn`**

keen is a C transpiler front-end. It keeps **all of C's semantics** and replaces only C's *surface syntax* with a Rust-style syntax. The transpiler is a lexer, a recursive-descent parser, and an emitter. It performs **no type checking, no borrow checking, and no safety enforcement**; the C compiler does all semantic analysis.

---

## Table of Contents

1. [Goals, non-goals, and the one rule](#1-goals-non-goals-and-the-one-rule)
2. [Lexical structure](#2-lexical-structure)
3. [Program structure and modules](#3-program-structure-and-modules)
4. [Types](#4-types)
5. [Items](#5-items)
6. [Statements](#6-statements)
7. [Expressions](#7-expressions)
8. [Attributes, conditional compilation, and raw C](#8-attributes-conditional-compilation-and-raw-c)
9. [Name mangling](#9-name-mangling)
10. [Emission model](#10-emission-model)
11. [Diagnostics](#11-diagnostics)
12. [C feature coverage](#12-c-feature-coverage)
13. [Complete example](#13-complete-example)
14. [Formal grammar (EBNF)](#14-formal-grammar-ebnf)
15. [Implementation notes](#15-implementation-notes)
16. [Out of scope and future work](#16-out-of-scope-and-future-work)

---

## 1. Goals, non-goals, and the one rule

### 1.1 Goals

- Every keen program has a direct C equivalent. Every keen construct maps to C constructs by local rewriting.
- Types read left to right (no declarator spiral).
- Rust-style conventions: `fn`, `let`, `struct`, `match`, `mod`, `impl`, `pub`, `as`, `use`, attributes, expression-tail returns, paths with `::`.
- Immutable by default, and the immutability is enforced by C itself (`const`).
- C's own semantics are preserved: integer promotion, undefined behavior, evaluation order, ABI, and linkage rules.

### 1.2 Non-goals

Ownership, borrowing, lifetimes, traits, generics, closures, pattern destructuring, `Option`/`Result`, slices, bounds checks, overflow checks, exhaustive-`match` checks, and any runtime library.

### 1.3 The one rule

> If translating a construct would require the transpiler to know the **type** of an expression, the construct is either omitted or made explicit in the syntax.

This rule explains most design decisions below (for example `->` is kept, `let x = e;` uses `__auto_type`, and method-call syntax is omitted).

### 1.4 Design decisions summary

| ID | Decision | Reason |
|----|----------|--------|
| D-1 | `let` is `const` by default; `let mut` is mutable | Free enforcement by the C compiler |
| D-2 | `.` accesses struct values, `->` accesses through pointers | No auto-deref, because that needs types |
| D-3 | Assignment is a statement, not an expression; no `++`/`--`; no ternary `?:` | Removes C's classic footguns without analysis |
| D-4 | `if`/`else` is also an expression, but only in a simple form | Replaces `?:` without needing statement expressions |
| D-5 | Tail expression of a function is its return value | Purely syntactic rewrite |
| D-6 | Items without `pub` are `static` (file-private); `pub` items have external linkage | Maps Rust privacy onto C linkage |
| D-7 | Types and functions are order-independent; value items are source-ordered | Cheap to implement (sorting + prototypes) |
| D-8 | Rust operator precedence; the emitter parenthesizes | Fixes C's `a & b == c` trap |
| D-9 | `!` is logical not; `~` is bitwise not | Rust's `!` is type-dependent; we have no types |
| D-10 | `match` is a statement that compiles to `switch` with automatic `break` | Familiar syntax, simple lowering |
| D-11 | Any unknown identifier in type position is an opaque C type name | Free interop with `FILE`, `va_list`, `jmp_buf`, and so on |

---

## 2. Lexical structure

### 2.1 Source text

Source files are UTF-8. Identifiers are ASCII. Whitespace (space, tab, CR, LF, FF) separates tokens and is otherwise insignificant. Newlines are **not** statement terminators.

### 2.2 Comments

```rust
// line comment
/* block comment (does NOT nest, same as C) */
/// doc comment (emitted as an ordinary // comment)
```

### 2.3 Identifiers

```
IDENT = [A-Za-z_][A-Za-z0-9_]*
```

- Identifiers beginning with `__` or `_` followed by an uppercase letter are reserved (inherited from C).
- A lone `_` is a placeholder token (used in `match` arms and `[T; _]`), not an identifier.
- **C-keyword collision:** an identifier that is a C keyword but not a keen keyword (`int`, `char`, `short`, `long`, `float`, `double`, `signed`, `unsigned`, `auto`, `register`, `inline`, `switch`, `case`, `default`, `restrict`-free names, and so on) is legal in keen and is emitted with a trailing underscore (`int` becomes `int_`). `void` is reserved only as a type name (see §4).

### 2.4 Keywords

```
alignof   as        atomic    break     const     continue  do
else      enum      extern    false     fn        for       goto
if        impl      in        let       loop      match     mod
mut       pub       restrict  return    Self      sizeof    static
static_assert       struct    true      type      typeof    union
use       volatile  while
```

Contextual (only special in one position): `_` (placeholder), `void` (type), `c` (only when followed by `!`).

### 2.5 Literals

| Kind | Syntax | Emitted as |
|------|--------|-----------|
| Decimal integer | `42`, `1_000_000` | `42`, `1000000` |
| Hex / octal / binary | `0xFF`, `0o17`, `0b1010` | `0xFF`, `017`, `0x0A` (binary is converted to hex) |
| Suffixed integer | `10u8`, `5usize`, `7i64` | `((uint8_t)10)`, `((size_t)5)`, `((int64_t)7)` |
| Float | `1.5`, `1e-3`, `2.0f32` | `1.5`, `1e-3`, `2.0f` |
| Boolean | `true`, `false` | `true`, `false` (`<stdbool.h>` is auto-included) |
| Character | `'a'`, `'\n'`, `'\x41'` | `'a'`, `'\n'`, `'\x41'` (type is C `int`, value is a byte) |
| String | `"hello\n"` | `"hello\n"` (type `*const c_char`) |
| Raw string | `r"C:\path"`, `r#"say "hi""#` | Re-escaped into a normal C string literal |

Notes:

- Unsuffixed integers keep C's typing rules (`int`, or wider if the value requires it).
- Escape sequences in character and string literals are **copied to C verbatim**, so any escape valid in C is valid in keen. Rust-only escapes such as `\u{...}` are not supported.
- Adjacent string literals are **not** concatenated by keen. Use the raw-C escape hatch if you need that.
- **Character literal vs. label:** `'x'` (a quote, one character or escape, a closing quote) is a character literal. A quote followed by an identifier with no closing quote (`'outer`) is a label token.

### 2.6 Operators and punctuation

```
+  -  *  /  %  &  |  ^  ~  !  <<  >>
&& ||  ==  !=  <  >  <=  >=
=  +=  -=  *=  /=  %=  &=  |=  ^=  <<=  >>=
->  .  ::  ,  ;  :  =>  ..  ..=  ...
(  )  [  ]  {  }  #  '
```

Not present: `++`, `--`, `?`, the comma operator, and `:?` (ternary).

### 2.7 Header paths

After the keyword `use`, the lexer accepts `<` characters up to `>` as a single header-name token (for example `<stdio.h>`), because `<` would otherwise lex as an operator.

---

## 3. Program structure and modules

### 3.1 Compilation unit

A `.kn` file is a list of **items** (§5). There are no statements at file scope.

### 3.2 `use`

```rust
use <stdio.h>;          // #include <stdio.h>
use "mylib.h";          // #include "mylib.h"
use net::open;          // import a name from module `net` (see 3.4)
use net::open as dial;  // import with a different local name
```

### 3.3 `mod`

```rust
mod net {                // inline module
    pub fn open() -> i32 { return 0; }
}
mod util;                // file module: refers to util.kn, emits #include "util.h"
```

- Modules are purely a **naming** mechanism (§9). They do not create real scopes in C.
- An inline `mod` may contain any item, including nested `mod`.
- Referring to an item uses a path: `net::open()`. Inside the same module, the plain name works.

### 3.4 Imports

`use path;` brings the last path segment into the local namespace; `use path as alias;` brings it in under `alias`. The transpiler keeps a per-module table mapping local names to mangled names and rewrites identifier tokens through it. A local variable or parameter with the same name as an import is a transpile error (E0301). This avoids full scope resolution while staying predictable.

### 3.5 Output modes

| Mode | Flag | Output |
|------|------|--------|
| Single file | default | One `.c` file containing everything |
| Split | `--split` | Per module: `name.h` (all `pub` items) plus `name.c` (everything else) |

---

## 4. Types

### 4.1 Primitive types

| keen | C | Notes |
|-------|---|-------|
| `i8` `i16` `i32` `i64` | `int8_t` … `int64_t` | `<stdint.h>` |
| `u8` `u16` `u32` `u64` | `uint8_t` … `uint64_t` | |
| `i128` `u128` | `__int128`, `unsigned __int128` | GCC/Clang |
| `isize` `usize` | `ptrdiff_t`, `size_t` | `<stddef.h>` |
| `f32` `f64` | `float`, `double` | |
| `bool` | `bool` | `<stdbool.h>` |
| `c_char` | `char` | Type of string-literal elements |
| `c_schar` `c_uchar` | `signed char`, `unsigned char` | |
| `c_short` `c_ushort` | `short`, `unsigned short` | |
| `c_int` `c_uint` | `int`, `unsigned int` | |
| `c_long` `c_ulong` | `long`, `unsigned long` | |
| `c_longlong` `c_ulonglong` | `long long`, `unsigned long long` | |
| `c_float` `c_double` `c_longdouble` | `float`, `double`, `long double` | |
| `void` | `void` | Only valid directly under a pointer: `*mut void` |

There is no `char` type; use `u8` for bytes and `c_char` for C strings.

### 4.2 Named types

Any path, or any identifier not otherwise defined, is a type name emitted as-is (after mangling for keen-declared types; D-11):

```rust
let f: *mut FILE = fopen("a.txt", "r");   // FILE comes from <stdio.h>
let mut ap: va_list;                        // va_list comes from <stdarg.h>
```

### 4.3 Pointer types

```
*mut T                 →  T*
*const T               →  const T*
*mut *const T          →  const T**
*restrict mut T        →  T* restrict
*restrict const T      →  const T* restrict
*mut void              →  void*
```

`mut`/`const` after `*` describes the **pointee**, matching Rust. (Constness of the pointer variable itself is controlled by `let` vs `let mut`.)

### 4.4 Array types

```
[T; N]       →  T[N]        // N is any constant expression (or a runtime value for a VLA)
[T; _]       →  T[]         // incomplete array (flexible array member, extern arrays, parameters)
[[T; 3]; 4]  →  T[4][3]     // array of 4 arrays of 3 T
*mut [T; 4]  →  T (*)[4]    // pointer to array
[*mut T; 4]  →  T* [4]      // array of pointers
```

### 4.5 Function pointer types

`fn(...) -> T` is a type that denotes a **pointer to function**:

```
fn(i32, i32) -> i32          →  int32_t (*)(int32_t, int32_t)
fn()                         →  void (*)(void)
fn(*const c_char, ...) -> i32 → int32_t (*)(const char*, ...)
fn(i32) -> !                 →  _Noreturn void (*)(int32_t)   // see 5.1
*mut fn(i32) -> i32          →  int32_t (**)(int32_t)
[fn(i32) -> i32; 8]          →  int32_t (*[8])(int32_t)
```

### 4.6 Qualifier prefixes

```
volatile T         →  volatile T
atomic T           →  _Atomic(T)
volatile *mut u32  →  volatile uint32_t*       // qualifier applies to the pointee type
```

`const` as a *type* qualifier on non-pointer types does not exist; use `let` (locals), `const` items, or `*const T`.

### 4.7 `typeof`

`typeof(expr)` is a type that emits `__typeof__(expr)` (or `typeof(expr)` in C23 mode).

### 4.8 Anonymous aggregates

`struct { ... }` and `union { ... }` may appear as field types (see §5.3).

### 4.9 Declarator emission

Because C declarators wrap *around* the name, the emitter has a function `declare(type, name) -> string` that works inside-out:

1. Start with the base type and the name: `int32_t` / `x`.
2. Walk the keen type from outermost to innermost:
   - `[T; N]`: append `[N]` to the name string (after parenthesizing if the name currently begins with `*`).
   - `*q T`: prepend `*` (plus qualifiers) to the name string; wrap in parentheses if the *next* step is an array or function.
   - `fn(P…) -> R`: parenthesize the name with `(*name)`, append `(P…)`, then continue with `R` as the new base.
3. The remaining base is emitted in front.

This one function is the only place C's syntax fights back.

---

## 5. Items

All items accept leading attributes (§8) and an optional `pub` (D-6).

### 5.1 Functions

```
{attr} [pub] fn NAME ( [params] ) [-> TYPE | -> !] ( BLOCK | ; )
```

```rust
fn add(a: i32, b: i32) -> i32 { a + b }
pub fn log(fmt: *const c_char, ...) -> i32 { /* variadic */ }
fn fatal(msg: *const c_char) -> ! { puts(msg); exit(1); }
fn hello() { puts("hi"); }                   // no `->` means void
pub fn main(argc: i32, argv: *mut *mut c_char) -> i32 { return 0; }
```

Rules:

- Omitting `->` means the return type is `void`. `-> !` means `_Noreturn void`.
- Parameters are always `name: type`. Parameter arrays are written as pointers (`a: *mut i32`), since C decays them anyway.
- `...` (variadic) must be last. Use `use <stdarg.h>;` and call the standard macros as ordinary calls: `va_start(ap, fmt)`.
- Linkage: `pub fn` is external; plain `fn` is emitted `static`. **`main` is always external** (and, because it is never mangled, `main` is only special at module root).
- A trailing `;` instead of a body declares a prototype. This is only needed for functions defined in raw C; keen functions are prototyped automatically (§10).
- **Tail returns (D-5):** if the function's last statement is an expression without a trailing `;`, it is emitted as `return expr;`. The rule propagates into tail-position `if`/`else`, `match` arms, and bare blocks:

  ```rust
  fn sign(x: i32) -> i32 {
      if x < 0 { -1 } else if x > 0 { 1 } else { 0 }
  }
  ```
  ```c
  static int32_t sign(int32_t x) {
      if ((x < 0)) { return (-1); } else if ((x > 0)) { return 1; } else { return 0; }
  }
  ```

- In a `void` function, a tail expression without `;` is simply emitted as an expression statement.
- `return expr;` and `return;` work as in C.

### 5.2 Constants and statics

```
{attr} [pub] const NAME : TYPE = EXPR ;
{attr} [pub] static [mut] NAME : TYPE [= EXPR] ;
```

```rust
const MAX: usize = 64;                 // static const size_t MAX = 64;
pub const PI: f64 = 3.14159;           // pub: in header as   static const double PI = 3.14159;
static mut counter: i32 = 0;           // static int32_t counter = 0;
pub static mut errors: i32 = 0;        // int32_t errors = 0;   (+ extern decl in header)
static greeting: *const c_char = "hi";   // static const char* const greeting = "hi";
```

- `static` without `mut` emits `const`.
- `static` is also valid inside function bodies (static storage duration, block scope).
- `const` items are emitted as `static const` (not `#define`), so they obey C scoping and typing rules and can be used in array sizes only where C allows `static const` (C23 `constexpr` mode: `--std=c23` emits `constexpr`).

### 5.3 Structs

```
{attr} [pub] struct NAME { fields } | struct NAME ;
```

```rust
struct Point { x: f32, y: f32 }

struct Header {
    kind: u8,
    flags: u32 : 3,        // bit-field of width 3
    len: u32 : 13,
    data: [u8; _],         // flexible array member
}

struct Variant {
    tag: i32,
    union {                // anonymous union member (C11)
        i: i32,
        f: f32,
    },
    inner: struct { a: i32, b: i32 },   // named member with an anonymous struct type
}

struct Opaque;             // incomplete type (only forward-declared)
```

Emission: `typedef struct Name Name; struct Name { ... };`. Field syntax is `name: type [: bitwidth]`, and the bit-width follows C's meaning.

### 5.4 Unions

Identical to structs, with the keyword `union`.

### 5.5 Enums

```
{attr} [pub] enum NAME [: INTTYPE] { VARIANT [= CONST_EXPR] , ... }
```

```rust
enum Color { Red, Green = 5, Blue }
enum Mode: u8 { Off, On }        // fixed underlying type (requires C23 output)
```

- Variants are **C-style** integer constants only (data-carrying variants are out of scope; see §16).
- Emission: `typedef enum Color Color; enum Color { Color_Red, Color_Green = 5, Color_Blue };`. Variants are prefixed with the enum name to avoid C's global enumerator namespace. Use them as `Color::Red`.

### 5.6 Type aliases

```rust
type Callback = fn(i32) -> i32;      // typedef int32_t (*Callback)(int32_t);
type Bytes = [u8; 16];               // typedef uint8_t Bytes[16];
type NodePtr = *mut Node;
```

### 5.7 `impl` blocks

```rust
impl Point {
    const ORIGIN_X: f32 = 0.0;
    fn len(p: *const Point) -> f32 { sqrtf(p->x * p->x + p->y * p->y) }
    pub fn new(x: f32, y: f32) -> Point { Point { x: x, y: y } }
}
```

- `impl` is only a **namespace prefix**. Each function becomes `Point_len`, `Point_new`, and so on. Constants become `Point_ORIGIN_X`.
- Call with a path: `Point::len(&p)`. There is no `.method()` syntax (it would require types).
- Inside the block, `Self` is the type name. There is no special `self` keyword; name the first parameter whatever you like.
- `impl` accepts only `fn` and `const` items.
- Multiple `impl` blocks for one type are allowed.

### 5.8 `extern` blocks

Declare functions or variables defined elsewhere (when a header is not used):

```rust
extern {
    fn puts(s: *const c_char) -> i32;
    fn printf(fmt: *const c_char, ...) -> i32;
    static mut errno: i32;
}
```

Emitted as ordinary `extern` declarations. Names inside `extern` are **never mangled**, even inside a `mod`.

### 5.9 `static_assert`

```rust
static_assert(sizeof(Point) == 8, "Point must be 8 bytes");
```

Emitted as `_Static_assert(...)` (or `static_assert` in C23). Valid as an item and as a statement.

### 5.10 `use`, `mod`

See §3.

---

## 6. Statements

A block is `{ stmt* [tail_expr] }`.

### 6.1 `let`

```
let [mut] NAME [: TYPE] [= EXPR] ;
```

| keen | C |
|-------|---|
| `let x: i32 = 5;` | `const int32_t x = 5;` |
| `let mut x: i32 = 5;` | `int32_t x = 5;` |
| `let x = foo();` | `const __auto_type x = foo();` |
| `let mut x = foo();` | `__auto_type x = foo();` |
| `let mut x: i32;` | `int32_t x;` (uninitialized, as in C) |
| `let p: *mut T = q;` | `T* const p = q;` |
| `let a: [i32; 3] = [1, 2, 3];` | `const int32_t a[3] = {1, 2, 3};` |
| `let s: Point = Point { x: 1.0, y: 2.0 };` | `const Point s = (Point){ .x = 1.0, .y = 2.0 };` |
| `let mut z: Point = Point {};` | `Point z = {0};` |
| `#[register] let mut i: i32 = 0;` | `register int32_t i = 0;` |

Rules:

- At least one of the type or the initializer must be present.
- Without a type, `__auto_type` (GCC/Clang) is emitted, or `auto` under `--std=c23`.
- Array literals and bare `{}` initializers need a declared type.
- There is no destructuring: the left side is a single identifier.
- Redeclaring a name in the **same scope** is a C error and is reported by the C compiler. Shadowing in an inner block is legal, as in C.
- Assigning to a non-`mut` variable produces a C "read-only variable" error (mapped to keen source via `#line`; §11).

### 6.2 Expression statements and assignment

```rust
foo(x);
x = 5;
x += 1;          // += -= *= /= %= &= |= ^= <<= >>=
*p = 7;
a[i] = b.f;
```

Assignment is a **statement** (D-3). It cannot appear inside another expression.

### 6.3 `if` / `else`

```rust
if cond { ... }
if cond { ... } else { ... }
if a { ... } else if b { ... } else { ... }
```

Parentheses around the condition are not used (and are legal but redundant); braces are mandatory. A struct literal directly in the condition must be parenthesized, as in Rust.

### 6.4 `while`, `loop`, `do`

```rust
while cond { ... }            // while (cond) { ... }
loop { ... }                  // for (;;) { ... }
do { ... } while cond;        // do { ... } while (cond);
```

### 6.5 `for`

Two forms, distinguished by lookahead (`IDENT [: type] in` selects the range form).

**Range form**

```rust
for i in 0..n { ... }         // for (ptrdiff_t i = 0; i < n; i++) { ... }
for i in 0..=n { ... }        // for (ptrdiff_t i = 0; i <= n; i++) { ... }
for i: usize in 0..n { ... }  // for (size_t i = 0; i < n; i++) { ... }
```

- The loop variable defaults to `isize`. Annotate it to change the type.
- Bounds are evaluated according to C's rules (the `n` expression is re-evaluated each iteration, since it is copied into the condition). To evaluate it once, assign it to a variable first.
- Only ascending ranges are provided; use the general form below for anything else.

**General (C-style) form**

```rust
for let mut i: i32 = 10; i > 0; i -= 2 { ... }
for ; ; { ... }
for p = head; p != NULL; p = p->next { ... }
for let mut i: i32 = 0; i < n; i += 1, j -= 1 { ... }
```

- `init` is a `let` (without its trailing `;` being part of the statement) or an assignment, or empty.
- `step` is a comma-separated list of assignments/calls (this is the only place the comma is allowed).
- Because it lowers to a real C `for`, `continue` correctly runs the step clause.

### 6.6 `match`

```
match EXPR { ARM, ARM, ... }
ARM = {attr} PATTERN { "|" PATTERN } "=>" ( BLOCK | EXPR_STMT ) [","]
PATTERN = CONST_EXPR | CONST_EXPR ".." "=" CONST_EXPR | PATH | "_"
```

```rust
match c {
    'a' | 'e' | 'i' | 'o' | 'u' => { vowels += 1; }
    '0'..='9'                   => { digits += 1; }
    Color::Red                  => puts("red"),
    #[fallthrough]
    1                           => { setup(); }
    2                           => { run(); }
    _                           => { puts("other"); }
}
```

Emitted as:

```c
switch (c) {
    case 'a': case 'e': case 'i': case 'o': case 'u': { vowels += 1; break; }
    case '0' ... '9': { digits += 1; break; }          /* GNU case range */
    case Color_Red: { puts("red"); break; }
    case 1: { setup(); /* falls through */ }
    case 2: { run(); break; }
    default: { puts("other"); break; }
}
```

Rules:

- **`match` is a statement** (D-10). It is also allowed in tail position (§5.1), where each arm's tail expression becomes a `return`.
- Patterns are integer or character **constants**, enum variant paths, constant paths, ranges `a..=b` (GNU/Clang extension; `--no-case-ranges` rejects them), alternatives joined with `|`, and `_` (becomes `default`).
- Every arm body is wrapped in `{ }` in the output, so arms may declare variables.
- A `break` is appended automatically unless the arm is marked with the attribute `#[fallthrough]`.
- **Exhaustiveness is not checked** (no safety enforcement). A `match` with no `_` arm simply has no `default`.
- **`break` inside a `match` arm targets the enclosing loop** (as in Rust), not the `switch`. The transpiler lowers such a `break` to `goto` a label emitted after the loop (see 6.8). `continue` needs no rewrite.

### 6.7 `return`

`return;` and `return expr;`, same as C.

### 6.8 `break`, `continue`, labels, `goto`

```rust
'outer: for i in 0..h {
    for j in 0..w {
        if grid[i][j] == 0 { continue 'outer; }
        if grid[i][j] == 9 { break 'outer; }
    }
}

goto 'cleanup;
...
'cleanup:
free(buf);
```

- `'name:` before a loop labels the loop. `break 'name;` / `continue 'name;` lower to `goto`s to two synthesized labels (`_l_name_brk` placed after the loop, `_l_name_cont` placed at the end of the loop body).
- A `break` inside a `match` that is inside a loop (with no closer loop in between) also uses a synthesized break label.
- `'name:` before a **non-loop** statement is a plain C label, and `goto 'name;` is a plain C `goto`. Labels are function-scoped, as in C.
- `break`/`continue` outside any loop, and `goto` to an undefined label, are transpile errors (E0401, E0402).

### 6.9 Local items

`struct`, `union`, `enum`, `type`, `const`, `static`, and `static_assert` may appear in blocks and follow C's block-scope rules (local types are *not* hoisted).

### 6.10 `c!` statement

`c!("raw C text");` emits the text verbatim (see §8.3).

---

## 7. Expressions

### 7.1 Precedence and associativity

From highest to lowest:

| Level | Operators | Associativity |
|------:|-----------|---------------|
| 1 | postfix: `f(x)`, `a[i]`, `s.f`, `p->f`, `P::name` | left |
| 2 | prefix: `-` `!` `~` `*` (deref) `&` (address-of) `sizeof` `alignof` | right |
| 3 | `as` | left |
| 4 | `*` `/` `%` | left |
| 5 | `+` `-` | left |
| 6 | `<<` `>>` | left |
| 7 | `&` | left |
| 8 | `^` | left |
| 9 | `\|` | left |
| 10 | `==` `!=` `<` `>` `<=` `>=` | **non-associative** |
| 11 | `&&` | left |
| 12 | `\|\|` | left |
| 13 | `..` `..=` | (only in `for` and `match`) |
| 14 | `=` `+=` `-=` … (statements only) | right |

Differences from C:

- Bitwise operators bind **tighter** than comparisons: `a & mask == 0` means `(a & mask) == 0` (D-8).
- `as` binds tighter than `*`: `x * y as u8` means `x * (y as u8)`.
- Comparison operators do not chain: `a < b < c` is a syntax error (E0201).
- The emitter **parenthesizes every non-atomic operand**, so the emitted C has the same meaning regardless of C's own precedence.

Operators, other than the differences above, have exactly C's meaning (including integer promotion, short-circuiting of `&&`/`||`, and undefined behavior).

### 7.2 Unary operators

| keen | C |
|-------|---|
| `-x` | `(-x)` |
| `!x` | `(!x)` — logical not only (D-9) |
| `~x` | `(~x)` — bitwise not |
| `*p` | `(*p)` |
| `&x` | `(&x)` |
| `sizeof(T)` | `sizeof(T)` — argument is a **type** |
| `alignof(T)` | `_Alignof(T)` |
| `offsetof(T, field)` | `offsetof(T, field)` (`<stddef.h>`) — ordinary call syntax |

To take `sizeof` of an expression, use `typeof`: `sizeof(typeof(x))`.

### 7.3 Casts

```rust
x as u8                  // ((uint8_t)x)
p as *mut Node           // ((Node*)p)
f as usize as *mut void  // chained: ((void*)((size_t)f))
```

### 7.4 Calls, indexing, member access

```rust
f(a, b)         // f(a, b)
a[i][j]         // a[i][j]
s.field         // s.field        (s is a struct/union value)
p->field        // p->field       (p is a pointer)
(*pp).field     // (*pp).field
fp(x)           // fp(x)          (calling a function pointer)
```

### 7.5 Paths

`a::b::c` names an item through modules and `impl`/`enum` prefixes. The path is mangled to a single C identifier (§9). Variables and parameters are plain identifiers.

### 7.6 Struct literals

```rust
Point { x: 1.0, y: 2.0 }      // (Point){ .x = 1.0, .y = 2.0 }
Point { x, y }                // (Point){ .x = x, .y = y }     (shorthand)
Point {}                      // (Point){0}
net::Addr { port: 80 }        // (net_Addr){ .port = 80 }
```

Unlike Rust, **missing fields are allowed** and are zero-initialized, because that is C's designated-initializer semantics. A struct literal cannot appear directly as the condition of `if`/`while`/`match` (parenthesize it).

### 7.7 Array literals

```rust
[1, 2, 3]            // {1, 2, 3}        (initializers only)
[0; 16]              // {0}              (only the zero value is allowed with `;`)
[1, 2, 3] as [i32; 3]   // (int32_t[3]){1, 2, 3}   (compound literal)
&([1, 2, 3] as [i32; 3])
```

- An array literal is valid as a `let`/`static`/`const` initializer, as a struct-literal field value, or as the operand of `as [T; N]`. It is otherwise an error (E0210), because the transpiler would need context-sensitive typing.
- `[x; N]` with a non-zero `x` is an error (E0211); fill with a loop.

### 7.8 `if` expression

```rust
let m = if a > b { a } else { b };      // const __auto_type m = ((a > b)) ? (a) : (b);
printf("%d\n", if x { 1 } else { 2 });
```

Each branch must be a **single expression** with no statements, and `else` is required (E0220). Chains `if … else if … else` are allowed. This replaces `?:`.

In tail position of a function, an `if` with statement bodies is a statement-level construct instead (§5.1).

### 7.9 `c!` expression

`c!("expr")` inserts raw C text as an expression (§8.3).

### 7.10 Evaluation order

Unchanged from C: unspecified evaluation order of operands and arguments, unsequenced modifications are undefined behavior, and so on. keen neither reorders nor introduces temporaries (except for the synthesized `break` labels and `for` range loops).

---

## 8. Attributes, conditional compilation, and raw C

### 8.1 Attributes

Syntax: `#[name]`, `#[name(arg, …)]`, or `#[name("string")]`, placed before an item, a `let`, a struct field, or a `match` arm.

| Attribute | Applies to | Emitted |
|-----------|-----------|---------|
| `#[inline]` | fn | `static inline` (private) or `inline` (pub) |
| `#[noinline]` | fn | `__attribute__((noinline))` |
| `#[noreturn]` | fn | `_Noreturn` (same as `-> !`) |
| `#[packed]` | struct, union | `__attribute__((packed))` |
| `#[align(N)]` | struct, union, let, static | `_Alignas(N)` |
| `#[thread_local]` | static | `_Thread_local` |
| `#[register]` | let | `register` |
| `#[weak]` | fn, static | `__attribute__((weak))` |
| `#[used]` | static, fn | `__attribute__((used))` |
| `#[section("name")]` | fn, static | `__attribute__((section("name")))` |
| `#[deprecated]` | any item | `__attribute__((deprecated))` |
| `#[unused]` | fn, let, static | `__attribute__((unused))` |
| `#[fallthrough]` | match arm | omit the automatic `break` |
| `#[attr(RAW)]` | any | `__attribute__((RAW))` — passes any GNU attribute through |
| `#[cfg(...)]` | any item, stmt | see 8.2 |

Unknown attribute names are an error (E0501); use `#[attr(...)]` for anything unlisted.

### 8.2 Conditional compilation

```rust
#[cfg(DEBUG)]
fn trace(msg: *const c_char) { puts(msg); }

#[cfg(not(DEBUG))]
fn trace(msg: *const c_char) {}
```

Emitted as `#ifdef NAME … #endif` and `#ifndef NAME … #endif` around the item or statement. For richer conditions, use `c!` (8.3). Identifier names are passed through unmangled.

### 8.3 Raw C escape hatch

`c!("...")` emits its string argument **verbatim**. Use it for anything keen does not model: `#define` macros, `_Generic`, `_Complex`, inline assembly, array designated initializers, token pasting, X-macros.

```rust
c!("#define ARRAY_LEN(a) (sizeof(a) / sizeof((a)[0]))");     // item position
fn f() -> i32 {
    c!("__asm__ volatile (\"nop\");");                         // statement position
    let x: i32 = c!("MY_MACRO(3)");                            // expression position
    x
}
```

- At item level, `c!` items are emitted in the **preamble** (after `use` includes and before any type), in source order, so macros defined there are visible everywhere.
- The attribute `#[late]` on a top-level `c!` item moves it after all prototypes and before function definitions.
- Raw strings (`r#"..."#`) are convenient for embedding C that contains quotes.
- User-defined C macros can be **called** as ordinary function-call syntax (`MY_MACRO(3)`); only macro *definitions* need `c!`.

---

## 9. Name mangling

keen names are flattened to C identifiers by joining path segments with `_`.

| keen | C | Rule |
|-------|---|------|
| `fn foo` at module root | `foo` | unchanged |
| `mod net { fn open }` | `net_open` | module prefix |
| `mod a { mod b { struct S } }` | `a_b_S` | nested modules |
| `impl Point { fn len }` | `Point_len` | impl prefix |
| `mod net { impl Addr { fn new } }` | `net_Addr_new` | combined |
| `enum Color { Red }` | `Color_Red` | enum prefix |
| `Self::len` inside `impl Point` | `Point_len` | `Self` resolves to the impl's type |
| `main` at module root | `main` | never mangled |
| `extern { fn puts }` | `puts` | extern names are never mangled |
| `int` (C-keyword collision) | `int_` | trailing underscore |

Collisions (for example `a_b::c` vs `a::b_c`) surface as C redefinition errors. The separator can be changed with `--mangle-sep=__`.

---

## 10. Emission model

The transpiler emits one C file (or an `.h`/`.c` pair per module in `--split` mode) in this fixed order. Steps 2–5 give D-7: **types and functions can be written in any order**.

1. **Preamble:** auto-included `<stdint.h>`, `<stddef.h>`, `<stdbool.h>`; the user's `use <...>`/`use "..."` includes in source order; then top-level `c!` items.
2. **Enums:** `typedef enum`, then definitions (no dependencies except a fixed-type enum's underlying type).
3. **Forward typedefs** for all structs and unions: `typedef struct Name Name;`.
4. **Type definitions in dependency order:** structs, unions, and `type` aliases form a graph, with an edge for each by-value use (struct field of struct type, array element type, alias target). Pointer-only references need just the forward typedef from step 3. The graph is topologically sorted; a by-value cycle is an error (E0601).
5. **Prototypes** for all functions and `extern` declarations for `pub static` items.
6. **Constants and statics** in **source order** (so a constant must be defined before another constant uses it, which is the C rule; D-7).
7. **Function definitions** in source order, then (if `#[late]` items are present) their raw C.

Other emission rules:

- Output is formatted with consistent indentation and one statement per line.
- Every binary and unary subexpression is parenthesized (use `--minimal-parens` to emit only where required).
- `#line N "file.kn"` directives are emitted before each statement and item (default on; `--no-line-directives` disables them).
- `--std=c11` (default) emits `__auto_type`/`_Static_assert`/`_Noreturn`/`_Alignas`; `--std=c23` emits `auto`/`static_assert`/`[[noreturn]]`/`alignas`/`constexpr`/typed enums.
- In `--split` mode, `pub` types, prototypes, `pub const` values, and `extern` declarations for `pub static` go into the `.h` (with include guards); everything else goes into the `.c`.

---

## 11. Diagnostics

The transpiler only checks things it can see without types:

| Code | Error |
|------|-------|
| E0101 | Unexpected token / unterminated literal or comment |
| E0201 | Chained comparison (`a < b < c`) |
| E0210 | Array literal without a type context |
| E0211 | `[x; N]` with non-zero `x` |
| E0220 | `if` expression without `else`, or with statements in a branch |
| E0301 | Local name shadows an import |
| E0401 | `break`/`continue` outside a loop, or unknown loop label |
| E0402 | `goto` to an undefined label |
| E0501 | Unknown attribute |
| E0601 | By-value cycle among struct/union/alias definitions |
| E0701 | Assignment used as an expression |

Everything else (type errors, undeclared names, assigning to a `let`, wrong argument counts) is reported by the **C compiler**. Because of `#line` directives, those errors point at the `.kn` source line.

---

## 12. C feature coverage

| C feature | keen |
|-----------|-------|
| Integer/float/bool types | `i8`…`u64`, `isize`, `usize`, `f32`, `f64`, `bool`, `c_*` |
| Pointers, arrays, VLAs | `*mut T`, `*const T`, `[T; N]` (N may be non-constant) |
| Function pointers | `fn(...) -> T` |
| `typedef` | `type` |
| `struct`, `union`, `enum` | same keywords; bit-fields, anonymous members, flexible array members supported |
| Compound literals | struct literals, `[…] as [T; N]` |
| Designated initializers (struct) | struct literals with field names |
| Designated initializers (array) | via `c!` |
| `static`, `extern`, `register`, `auto` | `static`, `extern { }`, `#[register]`, `let` |
| `_Thread_local` | `#[thread_local]` |
| `const`, `volatile`, `restrict`, `_Atomic` | `let`/`*const T`, `volatile T`, `*restrict mut T`, `atomic T` |
| `if`/`else`, `while`, `do-while`, `for` | `if`/`else`, `while`, `do … while`, `for` (range and C-style) |
| `switch` / `case` / `default` / fallthrough | `match` with `#[fallthrough]` |
| `break`, `continue`, `return` | same (with the `match` rule in 6.6) |
| `goto` and labels | `goto 'label;` and `'label:` |
| Ternary `?:` | `if … else …` expression |
| `++`, `--` | `+= 1`, `-= 1` (statement) |
| Assignment in expressions (`while ((c = getc(f)) != EOF)`) | split into a statement (use `loop` with a `break`) |
| Comma operator | only in the `for` step clause |
| Casts, `sizeof`, `_Alignof`, `offsetof` | `as`, `sizeof(T)`, `alignof(T)`, `offsetof(T, f)` |
| `typeof` | `typeof(expr)` |
| `_Static_assert` | `static_assert` |
| `_Noreturn` | `-> !` |
| `inline` | `#[inline]` |
| Variadic functions | `...` + `<stdarg.h>` |
| `#include` | `use <…>;` / `use "…";` |
| `#ifdef` / `#ifndef` | `#[cfg(NAME)]` / `#[cfg(not(NAME))]` |
| `#define`, `#if` expressions, token pasting, `_Generic`, `_Complex`, inline asm | `c!("…")` |
| Attributes (`__attribute__`) | `#[attr(…)]` and named shortcuts |

The only C idioms that must be **rewritten** (not just retranslated) are assignment-as-expression, `++`/`--` as expressions, the comma operator outside `for`, and implicit string-literal concatenation. All have a direct statement-level equivalent.

---

## 13. Complete example

### keen

```rust
use <stdio.h>;
use <stdlib.h>;
use <string.h>;

c!("#define MAX_NAME 32");

#[cfg(DEBUG)]
fn trace(msg: *const c_char) { fprintf(stderr, "trace: %s\n", msg); }
#[cfg(not(DEBUG))]
fn trace(msg: *const c_char) {}

enum Kind: u8 { Leaf, Branch }

struct Node {
    kind: Kind,
    name: [c_char; 32],
    left: *mut Node,
    right: *mut Node,
}

type Visit = fn(*mut Node) -> bool;

impl Node {
    pub fn new(kind: Kind, name: *const c_char) -> *mut Node {
        let n = calloc(1, sizeof(Node)) as *mut Node;
        n->kind = kind;
        strncpy(n->name, name, MAX_NAME - 1);
        n
    }

    pub fn walk(n: *mut Node, visit: Visit) -> usize {
        if n == NULL { return 0; }
        let mut count: usize = 0;
        if visit(n) { count += 1; }
        count += Node::walk(n->left, visit);
        count += Node::walk(n->right, visit);
        count
    }
}

fn print_leaf(n: *mut Node) -> bool {
    match n->kind {
        Kind::Leaf => {
            printf("leaf %s\n", n->name);
            true
        }
        _ => false,
    }
}

static mut visited: usize = 0;

fn classify(c: c_int) -> i32 {
    match c {
        '0'..='9'          => 1,
        'a'..='z' | 'A'..='Z' => 2,
        _                  => 0,
    }
}

pub fn main(argc: i32, argv: *mut *mut c_char) -> i32 {
    let root = Node::new(Kind::Branch, "root");
    root->left = Node::new(Kind::Leaf, "a");
    root->right = Node::new(Kind::Leaf, "b");

    trace("walking");
    visited = Node::walk(root, print_leaf);

    'scan: for i in 0..argc {
        for j in 0..3 {
            if classify(argv[i][j] as c_int) == 0 { continue 'scan; }
            if argv[i][j] == 'x' { break 'scan; }
        }
    }

    let mut total: i32 = 0;
    for let mut k: i32 = 10; k > 0; k -= 3 {
        total += k;
    }
    printf("%zu %d\n", visited, total);
    return 0;
}
```

### Emitted C (abridged, `--std=c11`)

```c
#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#define MAX_NAME 32

typedef enum Kind Kind;
enum Kind { Kind_Leaf, Kind_Branch };

typedef struct Node Node;
typedef bool (*Visit)(Node*);
struct Node {
    Kind kind;
    char name[32];
    Node* left;
    Node* right;
};

static void trace(const char* msg);
Node* Node_new(Kind kind, const char* name);
size_t Node_walk(Node* n, Visit visit);
static bool print_leaf(Node* n);
static int32_t classify(int c);
int main(int32_t argc, char** argv);

static size_t visited = 0;

#line 8 "tree.kn"
#ifndef DEBUG
static void trace(const char* msg) {}
#endif

Node* Node_new(Kind kind, const char* name) {
    Node* const n = (Node*)calloc(1, sizeof(Node));
    n->kind = kind;
    strncpy(n->name, name, (MAX_NAME - 1));
    return n;
}

size_t Node_walk(Node* n, Visit visit) {
    if ((n == NULL)) { return 0; }
    size_t count = 0;
    if (visit(n)) { count += 1; }
    count += Node_walk(n->left, visit);
    count += Node_walk(n->right, visit);
    return count;
}

static bool print_leaf(Node* n) {
    switch (n->kind) {
        case Kind_Leaf: { printf("leaf %s\n", n->name); return true; }
        default: { return false; }
    }
}

static int32_t classify(int c) {
    switch (c) {
        case '0' ... '9': { return 1; }
        case 'a' ... 'z': case 'A' ... 'Z': { return 2; }
        default: { return 0; }
    }
}

int main(int32_t argc, char** argv) {
    Node* const root = Node_new(Kind_Branch, "root");
    root->left = Node_new(Kind_Leaf, "a");
    root->right = Node_new(Kind_Leaf, "b");
    trace("walking");
    visited = Node_walk(root, print_leaf);
    for (ptrdiff_t i = 0; i < argc; i++) {
        for (ptrdiff_t j = 0; j < 3; j++) {
            if ((classify((int)argv[i][j]) == 0)) { goto _l_scan_cont; }
            if ((argv[i][j] == 'x')) { goto _l_scan_brk; }
        }
        _l_scan_cont: ;
    }
    _l_scan_brk: ;
    int32_t total = 0;
    for (int32_t k = 10; k > 0; k -= 3) {
        total += k;
    }
    printf("%zu %d\n", visited, total);
    return 0;
}
```

(In real output, expressions are fully parenthesized; abbreviated here for readability.)

---

## 14. Formal grammar (EBNF)

Notation: `{ x }` zero or more, `[ x ]` optional, `( a | b )` choice, quoted text is a literal token. Whitespace and comments are skipped between tokens.

```ebnf
(* ---------- Top level ---------- *)
file        = { item } ;

item        = { attribute } [ "pub" ]
              ( fn_item | struct_item | union_item | enum_item | type_item
              | const_item | static_item | mod_item | impl_item | use_item
              | extern_block | static_assert | raw_c ) ;

attribute   = "#" "[" IDENT [ "(" attr_args ")" ] "]" ;
attr_args   = { any token except unbalanced ")" } ;

(* ---------- Items ---------- *)
fn_item     = "fn" IDENT "(" [ params ] ")" [ "->" ( type | "!" ) ] ( block | ";" ) ;
params      = param { "," param } [ "," "..." ] [ "," ] | "..." ;
param       = IDENT ":" type ;

struct_item = "struct" IDENT ( "{" [ fields ] "}" | ";" ) ;
union_item  = "union" IDENT "{" [ fields ] "}" ;
fields      = field { "," field } [ "," ] ;
field       = { attribute }
              ( IDENT ":" type [ ":" const_expr ]
              | ( "struct" | "union" ) "{" [ fields ] "}" ) ;     (* anonymous member *)

enum_item   = "enum" IDENT [ ":" type ] "{" variant { "," variant } [ "," ] "}" ;
variant     = IDENT [ "=" const_expr ] ;

type_item   = "type" IDENT "=" type ";" ;
const_item  = "const" IDENT ":" type "=" expr ";" ;
static_item = "static" [ "mut" ] IDENT ":" type [ "=" expr ] ";" ;

mod_item    = "mod" IDENT ( "{" { item } "}" | ";" ) ;
impl_item   = "impl" IDENT "{" { { attribute } [ "pub" ] ( fn_item | const_item ) } "}" ;
use_item    = "use" ( HEADER | STRING | path [ "as" IDENT ] ) ";" ;
extern_block= "extern" "{" { ( extern_fn | extern_static ) } "}" ;
extern_fn   = "fn" IDENT "(" [ params ] ")" [ "->" type ] ";" ;
extern_static = "static" [ "mut" ] IDENT ":" type ";" ;
static_assert = "static_assert" "(" expr [ "," STRING ] ")" ";" ;
raw_c       = "c" "!" "(" STRING ")" ";" ;

(* ---------- Types ---------- *)
type        = "*" [ "restrict" ] ( "mut" | "const" ) type
            | "[" type ";" ( expr | "_" ) "]"
            | "fn" "(" [ type_list ] ")" [ "->" ( type | "!" ) ]
            | ( "volatile" | "atomic" ) type
            | "typeof" "(" expr ")"
            | ( "struct" | "union" ) "{" [ fields ] "}"
            | "(" type ")"
            | path ;
type_list   = ( type { "," type } [ "," "..." ] [ "," ] ) | "..." ;
path        = IDENT { "::" IDENT } ;

(* ---------- Statements ---------- *)
block       = "{" { stmt } [ expr ] "}" ;

stmt        = ";"
            | { attribute } let_stmt
            | { attribute } ( const_item | static_item | struct_item | union_item
                            | enum_item | type_item | static_assert )
            | assign_stmt
            | expr ";"
            | raw_c
            | if_stmt | while_stmt | loop_stmt | do_stmt | for_stmt | match_stmt
            | block
            | "return" [ expr ] ";"
            | "break" [ LABEL ] ";"
            | "continue" [ LABEL ] ";"
            | "goto" LABEL ";"
            | LABEL ":" stmt ;

let_stmt    = "let" [ "mut" ] IDENT [ ":" type ] [ "=" expr ] ";" ;
assign_stmt = expr assign_op expr ";" ;
assign_op   = "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" ;

if_stmt     = "if" expr block [ "else" ( if_stmt | block ) ] ;
while_stmt  = [ LABEL ":" ] "while" expr block ;
loop_stmt   = [ LABEL ":" ] "loop" block ;
do_stmt     = [ LABEL ":" ] "do" block "while" expr ";" ;

for_stmt    = [ LABEL ":" ] "for" ( range_for | c_for ) block ;
range_for   = IDENT [ ":" type ] "in" expr ( ".." | "..=" ) expr ;
c_for       = [ for_init ] ";" [ expr ] ";" [ step { "," step } ] ;
for_init    = "let" [ "mut" ] IDENT [ ":" type ] [ "=" expr ] | expr assign_op expr ;
step        = expr [ assign_op expr ] ;

match_stmt  = "match" expr "{" { arm } "}" ;
arm         = { attribute } pattern { "|" pattern } "=>" ( block | expr ";" | expr "," ) [ "," ] ;
pattern     = "_" | const_expr [ ( ".." | "..=" ) const_expr ] ;

(* ---------- Expressions (lowest to highest precedence) ---------- *)
expr        = or_expr ;
or_expr     = and_expr { "||" and_expr } ;
and_expr    = cmp_expr { "&&" cmp_expr } ;
cmp_expr    = bor_expr [ ( "==" | "!=" | "<" | ">" | "<=" | ">=" ) bor_expr ] ;   (* non-assoc *)
bor_expr    = bxor_expr { "|" bxor_expr } ;
bxor_expr   = band_expr { "^" band_expr } ;
band_expr   = shift_expr { "&" shift_expr } ;
shift_expr  = add_expr { ( "<<" | ">>" ) add_expr } ;
add_expr    = mul_expr { ( "+" | "-" ) mul_expr } ;
mul_expr    = cast_expr { ( "*" | "/" | "%" ) cast_expr } ;
cast_expr   = unary_expr { "as" type } ;
unary_expr  = ( "-" | "!" | "~" | "*" | "&" ) unary_expr
            | "sizeof" "(" type ")" | "alignof" "(" type ")"
            | postfix_expr ;
postfix_expr= primary { "(" [ args ] ")" | "[" expr "]" | "." IDENT | "->" IDENT } ;
args        = expr { "," expr } [ "," ] ;

primary     = INT | FLOAT | CHAR | STRING | RAW_STRING | "true" | "false"
            | path
            | struct_lit
            | array_lit
            | if_expr
            | raw_c_expr
            | "(" expr ")" ;

struct_lit  = path "{" [ field_init { "," field_init } [ "," ] ] "}" ;
field_init  = IDENT [ ":" expr ] ;
array_lit   = "[" ( expr { "," expr } [ "," ] | expr ";" expr ) "]" ;
if_expr     = "if" expr "{" expr "}" "else" ( "{" expr "}" | if_expr ) ;
raw_c_expr  = "c" "!" "(" STRING ")" ;

(* ---------- Tokens ---------- *)
LABEL       = "'" IDENT ;
HEADER      = "<" { any character except ">" } ">" ;     (* only after `use` *)
const_expr  = expr ;                                       (* must be a C constant expression *)
```

Semantic restrictions not expressed in the grammar: assignment statements may not appear in expression position (E0701); `if_expr` branches are single expressions (E0220); struct literals are not allowed directly in `if`/`while`/`match` heads without parentheses; a tail `expr` in a `block` is only meaningful at function-body, `if`, or `match`-arm position.

---

## 15. Implementation notes

### 15.1 Pipeline

```
source.kn ─▶ lexer ─▶ parser ─▶ AST ─▶ [passes] ─▶ emitter ─▶ source.c (+ .h)
```

Passes (each small and independent):

1. **Import/mangle pass:** build the module table, resolve `use` aliases, and compute mangled names for each item (§9).
2. **Control-flow pass:** track the loop/`match` stack, assign `_l_<name>` labels, rewrite `break`/`continue` inside `match` and labeled forms (§6.8).
3. **Tail-return pass:** push `return` into tail positions of function bodies (§5.1).
4. **Type-dependency pass:** build the by-value dependency graph and topologically sort type items (§10 step 4).

The AST needs **no type annotations** beyond what the user wrote, and the only symbol information kept is item names (for mangling) and local names (for the import-shadow check).

### 15.2 Emitter

- A `declare(type, name)` function (§4.9) is shared by every declaration site: locals, parameters, fields, aliases, casts, and `sizeof`.
- An expression printer parenthesizes by default; precedence-aware minimal printing is an optional flag.
- A line-map table drives `#line` directives.

### 15.3 CLI

```
keen input.kn [-o out.c] [--split] [--std=c11|c23]
               [--no-line-directives] [--minimal-parens]
               [--mangle-sep=_] [--no-case-ranges]
```

### 15.4 Testing strategy

1. **Round-trip golden tests:** `.kn` input and expected `.c` output.
2. **Compile tests:** every golden `.c` must compile with `-std=c11 -Wall -Wextra -Werror` on GCC and Clang.
3. **Differential tests:** write the same small programs in C and keen and compare program output.
4. **Negative tests:** every diagnostic code in §11 has at least one failing input.

---

## 16. Out of scope and future work

Deliberately excluded to keep the transpiler lightweight:

- **Generics, traits, closures, lifetimes, borrow checking, `unsafe`** (everything is "unsafe", so the keyword would be redundant).
- **Data-carrying enums.** The most natural next addition. It would desugar to a tagged union plus a `match` over the tag, with no type checking required.
- **Slices, `Option`, `Result`.** These imply runtime types; users can define their own structs.
- **Method-call syntax (`x.len()`).** Requires knowing the type of `x`.
- **Auto-deref on `.`.** Same reason; `->` is kept.
- **Pattern destructuring.** `let (a, b) = …` and struct patterns need tuple/struct typing.
- **Macros.** Only `c!`. A Rust-like `macro_rules!` is a large feature.

Possible small extensions (each cheap, none required):

- An opt-in flag permitting assignment as an expression (`--allow-assign-expr`) for `while ((c = getc(f)) != EOF)` idioms.
- `for i in (a..b).rev()` and `.step_by(n)` lowered syntactically.
- `defer`-style cleanup lowered to `goto`.
- A `Self`-typed first parameter sugar (`self`) with `Type::method(&x)` call syntax unchanged.

---

*End of specification.*

---

## Appendix A. Implementation notes and deviations (keen 0.1)

The reference implementation in this repository follows the specification above, with the
following clarifications and small deviations, found while implementing and testing it
against GCC 13.

1. **`-> !` in function-pointer types** emits plain `void`. `_Noreturn` is a function
   specifier and cannot appear inside a C type name. (`fn f() -> !` definitions and prototypes
   still get `_Noreturn` / `[[noreturn]]`.)
2. **`volatile`/`atomic` prefixes qualify the type that follows**, uniformly. So
   `*mut volatile u32` is a pointer to volatile `u32` (`volatile uint32_t*`, the MMIO form),
   while `volatile *mut u32` is a volatile pointer (`uint32_t* volatile`). The §4.6 table row
   for `volatile *mut u32` is superseded by this rule.
3. **Brace initializers.** A struct or array literal that is *directly* the initializer of a
   `let`/`static`/`const` with an explicit type is emitted as a brace initializer
   (`const P a = { .x = 1 };`), which is valid for both automatic and static storage. A struct
   literal anywhere else, or with an inferred `let` type, is a compound literal.
4. **Const items** are always `static const` (no `constexpr` in `--std=c23`), because C23
   `constexpr` cannot be used for pointers or non-constant initializers.
5. **Enums** are emitted as the definition followed by the typedef (ISO C has no forward
   references to enums without a fixed underlying type). `enum Name: u8 { .. }` needs
   `--std=c23` (or a GNU-mode compiler).
6. **Top-level expressions are not parenthesized**; only operands are (`return -1;`,
   `if (x < 0)`), so the output stays readable. `--minimal-parens` parenthesizes only where C's
   precedence requires it.
7. **Types as macro arguments.** A bare primitive type name used as a call argument is emitted
   as its C type (`va_arg(ap, i32)` becomes `va_arg(ap, int32_t)`). For more complex types use
   a `type` alias or `c!`.
8. **`match` has no exhaustiveness check.** A `match` in return position without a `_` arm may
   make the C compiler warn that control reaches the end of a non-void function.
9. **`main`** gets no prototype and is never `static`.
10. **`#[inline]` on a `pub fn`** emits `inline` on the definition only; the prototype is plain,
    so the compiler produces an external definition (C99 inline semantics).
11. **`for i in a..b`** re-evaluates `b` on every iteration (it is copied into the C condition).
12. **`--module NAME`** prefixes every top-level name of a file with `NAME` so that a file module
    (`mod util;` in another file) can be compiled separately with `--split`; `mod util;` itself
    emits `#include "util.h"`.
13. **`c!`** processes ordinary string escapes (`\"`, `\n`, `\\`, `\t`); use a raw string
    (`c!(r#"..."#)`) to avoid escaping.
14. **Decimal literals** have leading zeros stripped (`007` is `7`, never C octal); `0b…` literals
    are converted to hex.
15. **Extra diagnostics:** `E0102` (a C type keyword such as `int` used in type position) and
    `E0801` (range pattern with `--no-case-ranges`).

## Appendix B. Tooling (keen 0.2)

- **Projects.** `keen.toml` (`[package]`, `[build]`) marks a project root; build-ish commands find it
  by walking up from the current directory and run from there. Output goes to
  `target/<debug|release>/<name>` (tests: `<name>-test`), generated C to `target/keen/<profile>`.
  Single files build to `./<stem>` with generated C in `.keen/<stem>` (or a temp directory for
  `run`, `test` and `check`).
- **File modules.** `mod name;` loads `name.kn` or `name/mod.kn` next to the declaring file (for a
  non-root file `foo.kn`, also `foo/name.kn`). Each file module is transpiled with `--module a::b`
  and `--split`, producing `name.h`/`name.c`; module names must be unique because they become header
  names. A file refers to a sibling at the crate root with `use "other.h";` plus
  `crate::other::item`.
- **`#[test]`.** A test is a bodied function with no parameters. It is omitted from normal builds. In
  `keen test` the user's `main` is dropped and replaced by a runner; every file registers its tests
  through a generated `keen_register_<stem>()`.
- **Formatter.** `keen fmt` is token based and layout preserving; see the README for the rules. It
  refuses files that do not parse and verifies that its output lexes to the same tokens.
- **Line mapping.** `#line` directives are emitted only where the C compiler's running line count
  would otherwise be wrong, so diagnostics point at the exact `.kn` line.
- **Diagnostics.** Every error carries a code; `keen explain <code>` documents it. New in 0.2:
  `E0502` (invalid `#[test]` function) and `E0901` (internal formatter error).
- **Line endings.** Sources may use LF or CRLF; the lexer normalizes `\r\n` to `\n` before anything
  else, so a multi-line raw string or `c!` block has the same value in both. `keen fmt` keeps the
  file's existing style. Generated C and generated project files use LF.

## Appendix C. Installer (whet, keen 0.3)

- **whet** is the installer and toolchain manager (a separate binary in the same package); see
  `docs/INSTALL.md` for the user guide. It is dependency-free: SHA-256 is implemented in Rust,
  downloads shell out to `curl`/`wget` (and `file://` URLs are copied directly), archives are
  unpacked with the system `tar`.
- **Home.** `~/.keen` (or `$KEEN_HOME`, `--prefix`): `bin/keen` is a symlink into
  `toolchains/<version>/bin/keen`, so switching the default is atomic and costs nothing at run time.
  `settings.toml` records the default toolchain, the dist server and every file outside the home that
  was created or edited, so `whet self uninstall` can undo all of it.
- **Release layout.** `latest`, `keen/<ver>/keen-<ver>-<target>.tar.gz`, `whet/<ver>/whet-<target>`,
  each artifact next to a `.sha256`. A toolchain archive unpacks to `keen-<ver>-<target>/bin/keen`
  (+ `share/`). Archives containing absolute or `..` paths are refused.
- **Safety.** Every download is verified; `http://` is accepted only for loopback hosts (or with
  `WHET_ALLOW_HTTP=1`); profile edits are a delimited, idempotent block; whet refuses to delete a
  directory that has no `settings.toml`.
- **Bootstrap.** `install.sh` (POSIX sh) detects the platform, downloads and verifies `whet`, then
  runs `whet init` with the user's arguments, reading prompts from `/dev/tty` when invoked through a
  pipe. `install.ps1` is the PowerShell equivalent (untested on real Windows).
- **Limits.** Automatic PATH setup is Unix-only for now; per-project toolchain pinning is not
  implemented (use `whet run <version> keen ...`).
