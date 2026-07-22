use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

use dashmap::DashMap;

// ─── Symbol ───

/// Interned string identifier.
///
/// # Examples
///
/// ```
/// use maria::intern::Symbol;
///
/// let sym = Symbol::intern("hello");
/// assert_eq!(sym.as_str(), "hello");
/// ```
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct Symbol(u32);

impl Symbol {
    /// The empty symbol.
    pub const EMPTY: Symbol = Symbol(0);

    /// Create a new symbol for the given string.
    /// Interns the string if not already present.
    pub fn intern(s: &str) -> Self {
        table().intern(s)
    }

    /// Get the string representation of this symbol.
    pub fn as_str(&self) -> &'static str {
        table().get(self.0)
    }

    /// Get the raw index.
    pub fn index(self) -> u32 {
        self.0
    }

    /// Number of unique symbols interned so far.
    pub fn count() -> u32 {
        table().strings.lock().len() as u32
    }

    /// Create a symbol from a raw index (unsafe — only for deserialization).
    pub unsafe fn from_index(idx: u32) -> Self {
        Symbol(idx)
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Hash for Symbol {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl From<&str> for Symbol {
    fn from(s: &str) -> Self {
        Symbol::intern(s)
    }
}

impl PartialEq<&str> for Symbol {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<str> for Symbol {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl AsRef<str> for Symbol {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

// ─── Global String Table ───

/// Thread-safe string table with O(1) DashMap lookup.
/// Strings stored as owned String — no leak overhead.
struct StringTable {
    /// Indexed storage — ensures stable u32 handles
    strings: parking_lot::Mutex<Vec<String>>,
    /// O(1) hash-based lookup — string → u32 index
    lookup: DashMap<String, u32, fxhash::FxBuildHasher>,
}

fn table() -> &'static StringTable {
    static TABLE: OnceLock<StringTable> = OnceLock::new();
    TABLE.get_or_init(|| {
        let table = StringTable::new();
        // Populate 54 pre-defined symbols (index 0 = empty, 1..=53 = keywords)
        let prelude = vec![
            "",           // 0  = EMPTY
            "logic",      // 1
            "wire",       // 2
            "reg",        // 3
            "int",        // 4
            "integer",    // 5
            "bit",        // 6
            "byte",       // 7
            "shortint",   // 8
            "longint",    // 9
            "real",       // 10
            "time",       // 11
            "string",     // 12
            "input",      // 13
            "output",     // 14
            "inout",      // 15
            "module",     // 16
            "endmodule",  // 17
            "assign",     // 18
            "always",     // 19
            "initial",    // 20
            "begin",      // 21
            "end",        // 22
            "if",         // 23
            "else",       // 24
            "for",        // 25
            "while",      // 26
            "case",       // 27
            "parameter",  // 28
            "localparam", // 29
            "function",   // 30
            "endfunction",// 31
            "task",       // 32
            "endtask",    // 33
            "generate",   // 34
            "endgenerate",// 35
            "class",      // 36
            "endclass",   // 37
            "package",    // 38
            "endpackage", // 39
            "interface",  // 40
            "endinterface",// 41
            "fork",       // 42
            "join",       // 43
            "typedef",    // 44
            "enum",       // 45
            "struct",     // 46
            "union",      // 47
            "import",     // 48
            "bind",       // 49
            "uvm_object", // 50
            "uvm_component", // 51
            "uvm_test",   // 52
            "process",    // 53
        ];
        for s in &prelude {
            table.intern_fast(s);
        }
        table
    })
}

impl StringTable {
    fn new() -> Self {
        StringTable {
            strings: parking_lot::Mutex::new(Vec::with_capacity(10240)),
            lookup: DashMap::with_hasher(fxhash::FxBuildHasher::default()),
        }
    }

    /// Fast intern — assumes string not yet interned (used for pre-population).
    fn intern_fast(&self, s: &str) -> Symbol {
        let mut strings = self.strings.lock();
        let id = strings.len() as u32;
        let owned = s.to_string();
        self.lookup.insert(owned.clone(), id);
        strings.push(owned);
        Symbol(id)
    }

    fn intern(&self, s: &str) -> Symbol {
        // Fast path: O(1) DashMap lookup
        if let Some(sym) = self.lookup.get(s) {
            return Symbol(*sym);
        }

        // Slow path: allocate new entry
        let mut strings = self.strings.lock();
        // Double-check after acquiring lock (prevent race)
        for (i, existing) in strings.iter().enumerate() {
            if existing == s {
                self.lookup.insert(existing.clone(), i as u32);
                return Symbol(i as u32);
            }
        }
        let id = strings.len() as u32;
        let owned = s.to_string();
        self.lookup.insert(owned.clone(), id);
        strings.push(owned);
        Symbol(id)
    }

    fn get(&self, id: u32) -> &'static str {
        let strings = self.strings.lock();
        // Safe: strings are append-only, never removed, table is static
        unsafe { std::mem::transmute::<&str, &'static str>(&strings[id as usize]) }
    }
}

// ─── Pre-populated Symbols ───

/// Pre-populated symbols untuk keywords umum.
pub mod prelude {
    use super::Symbol;

    pub const S_EMPTY: Symbol = Symbol(0);
    pub const S_LOGIC: Symbol = Symbol(1);
    pub const S_WIRE: Symbol = Symbol(2);
    pub const S_REG: Symbol = Symbol(3);
    pub const S_INT: Symbol = Symbol(4);
    pub const S_INTEGER: Symbol = Symbol(5);
    pub const S_BIT: Symbol = Symbol(6);
    pub const S_BYTE: Symbol = Symbol(7);
    pub const S_SHORTINT: Symbol = Symbol(8);
    pub const S_LONGINT: Symbol = Symbol(9);
    pub const S_REAL: Symbol = Symbol(10);
    pub const S_TIME: Symbol = Symbol(11);
    pub const S_STRING: Symbol = Symbol(12);
    pub const S_INPUT: Symbol = Symbol(13);
    pub const S_OUTPUT: Symbol = Symbol(14);
    pub const S_INOUT: Symbol = Symbol(15);
    pub const S_MODULE: Symbol = Symbol(16);
    pub const S_ENDMODULE: Symbol = Symbol(17);
    pub const S_ASSIGN: Symbol = Symbol(18);
    pub const S_ALWAYS: Symbol = Symbol(19);
    pub const S_INITIAL: Symbol = Symbol(20);
    pub const S_BEGIN: Symbol = Symbol(21);
    pub const S_END: Symbol = Symbol(22);
    pub const S_IF: Symbol = Symbol(23);
    pub const S_ELSE: Symbol = Symbol(24);
    pub const S_FOR: Symbol = Symbol(25);
    pub const S_WHILE: Symbol = Symbol(26);
    pub const S_CASE: Symbol = Symbol(27);
    pub const S_PARAMETER: Symbol = Symbol(28);
    pub const S_LOCALPARAM: Symbol = Symbol(29);
    pub const S_FUNCTION: Symbol = Symbol(30);
    pub const S_ENDFUNCTION: Symbol = Symbol(31);
    pub const S_TASK: Symbol = Symbol(32);
    pub const S_ENDTASK: Symbol = Symbol(33);
    pub const S_GENERATE: Symbol = Symbol(34);
    pub const S_ENDGENERATE: Symbol = Symbol(35);
    pub const S_CLASS: Symbol = Symbol(36);
    pub const S_ENDCLASS: Symbol = Symbol(37);
    pub const S_PACKAGE: Symbol = Symbol(38);
    pub const S_ENDPACKAGE: Symbol = Symbol(39);
    pub const S_INTERFACE: Symbol = Symbol(40);
    pub const S_ENDINTERFACE: Symbol = Symbol(41);
    pub const S_FORK: Symbol = Symbol(42);
    pub const S_JOIN: Symbol = Symbol(43);
    pub const S_TYPEDEF: Symbol = Symbol(44);
    pub const S_ENUM: Symbol = Symbol(45);
    pub const S_STRUCT: Symbol = Symbol(46);
    pub const S_UNION: Symbol = Symbol(47);
    pub const S_IMPORT: Symbol = Symbol(48);
    pub const S_BIND: Symbol = Symbol(49);
    pub const S_UVM_OBJECT: Symbol = Symbol(50);
    pub const S_UVM_COMPONENT: Symbol = Symbol(51);
    pub const S_UVM_TEST: Symbol = Symbol(52);
    pub const S_PROCESS: Symbol = Symbol(53);
}

/// Initialize the global string table.
pub fn init_string_table() {
    let _ = table();
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intern_basic() {
        let s1 = Symbol::intern("hello");
        let s2 = Symbol::intern("hello");
        let s3 = Symbol::intern("world");

        assert_eq!(s1, s2);
        assert_ne!(s1, s3);
        assert_eq!(s1.as_str(), "hello");
        assert_eq!(s3.as_str(), "world");
    }

    #[test]
    fn test_intern_equality() {
        let a = Symbol::intern("foo_bar_123");
        let b = Symbol::intern("foo_bar_123");
        let c = Symbol::intern("foo_bar_124");

        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_intern_empty() {
        let e = Symbol::EMPTY;
        assert_eq!(e.as_str(), "");
    }

    #[test]
    fn test_symbol_display() {
        let s = Symbol::intern("display_test");
        assert_eq!(format!("{}", s), "display_test");
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
        let mut handles = Vec::new();
        for t in 0..8 {
            handles.push(thread::spawn(move || {
                for i in 0..300 {
                    let s = format!("thread{}_{}", t, i);
                    let sym = Symbol::intern(&s);
                    assert_eq!(sym.as_str(), s.as_str(),
                        "Mismatch for thread{}_[{}]", t, i);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn test_prelude_symbols() {
        assert_eq!(prelude::S_LOGIC.as_str(), "logic");
        assert_eq!(prelude::S_MODULE.as_str(), "module");
        assert_eq!(prelude::S_UVM_OBJECT.as_str(), "uvm_object");
    }

    #[test]
    fn test_dashmap_intern_speed() {
        let start = std::time::Instant::now();
        for i in 0..10000 {
            let s = format!("fast_intern_{}", i);
            let _sym = Symbol::intern(&s);
        }
        let elapsed = start.elapsed();
        eprintln!("Interned 10000 strings in {:?}", elapsed);
        // Should be fast — O(1) per intern now
        // (Generous threshold for debug builds)
        assert!(elapsed.as_secs() < 30, "too slow: {:?}", elapsed);
    }
}
