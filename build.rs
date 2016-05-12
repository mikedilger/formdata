#[cfg(feature = "rust-stable")]
extern crate syntex;
#[cfg(feature = "rust-stable")]
extern crate serde_codegen;

#[cfg(feature = "rust-stable")]
mod inner {

    use std::env;
    use std::path::Path;

    pub fn main() {
        let out_dir = env::var_os("OUT_DIR").unwrap();

        let src = Path::new("src/lib.rs.in");
        let dst = Path::new(&out_dir).join("lib.rs");

        let mut registry = ::syntex::Registry::new();

        ::serde_codegen::register(&mut registry);
        registry.expand("", &src, &dst).unwrap();
    }
}

#[cfg(feature = "rust-nightly")]
mod inner {
    pub fn main() {}
}

pub fn main() {
    inner::main()
}
