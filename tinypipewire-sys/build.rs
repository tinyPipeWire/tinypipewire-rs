use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=src/bindings/pregenerated.rs");
    println!("cargo:rerun-if-env-changed=TINYPIPEWIRE_SYS_UPDATE_BINDINGS");
    println!("cargo:rerun-if-env-changed=TINYPIPEWIRE_SYS_HEADERS_ONLY");

    // docs.rs builds in a sandbox with no PipeWire, so nothing is linked there
    // and the committed bindings stand in for a generated set.
    let headers_only = env::var_os("TINYPIPEWIRE_SYS_HEADERS_ONLY");
    let mut include_paths = if env::var_os("DOCS_RS").is_some() {
        Vec::new()
    } else if let Some(dir) = headers_only {
        // Maintainer escape hatch: bind these headers and emit no link flags,
        // so the committed bindings can be refreshed anywhere. "1" means the
        // vendored submodule's own headers.
        vec![if dir == "1" {
            vendor_dir().join("include")
        } else {
            PathBuf::from(dir)
        }]
    } else {
        link()
    };
    include_paths.extend(version_header_fallback());

    write_bindings(&include_paths);
}

/// Writes `tpw/tpw_version.h` from the submodule's template into `OUT_DIR`
/// and returns the include directory holding it.
///
/// Meson generates that header at build time, so it is missing whenever the
/// headers are read straight out of a source tree. The directory goes last on
/// the include path, so an installed copy still wins.
fn version_header_fallback() -> Option<PathBuf> {
    let src = vendor_dir();
    let template = std::fs::read_to_string(src.join("include/tpw/tpw_version.h.in")).ok()?;
    let project = std::fs::read_to_string(src.join("meson.build")).ok()?;

    let version = project
        .lines()
        .find_map(|line| line.trim().strip_prefix("version: '")?.strip_suffix("',"))?;
    let mut parts = version.split('.');
    let (major, minor, patch) = (parts.next()?, parts.next()?, parts.next()?);

    let dir = PathBuf::from(env::var("OUT_DIR").unwrap()).join("version-include/tpw");
    std::fs::create_dir_all(&dir).ok()?;
    std::fs::write(
        dir.join("tpw_version.h"),
        template
            .replace("@TPW_VERSION_MAJOR@", major)
            .replace("@TPW_VERSION_MINOR@", minor)
            .replace("@TPW_VERSION_PATCH@", patch),
    )
    .ok()?;
    dir.parent().map(Path::to_path_buf)
}

/// Makes the C library available to the linker and returns the include
/// directories its headers live under.
fn link() -> Vec<PathBuf> {
    if cfg!(feature = "vendored") {
        return build_vendored();
    }

    match pkg_config::Config::new()
        .atleast_version("0.9.0")
        .probe("tinypipewire")
    {
        Ok(lib) => lib.include_paths,
        Err(err) => {
            println!("cargo:warning=pkg-config could not find tinypipewire ({err}); building the vendored copy instead");
            build_vendored()
        }
    }
}

/// Builds the pinned C sources under `vendor/` with meson and links them
/// statically. PipeWire itself still has to come from the system.
fn build_vendored() -> Vec<PathBuf> {
    let src = vendor_dir();
    assert!(
        src.join("meson.build").is_file(),
        "vendor/tinypipewire is empty; run `git submodule update --init`"
    );

    let build_dir = PathBuf::from(env::var("OUT_DIR").unwrap()).join("vendor-build");
    if !build_dir.join("build.ninja").is_file() {
        run(Command::new("meson").args([
            "setup",
            "--default-library=static",
            "--buildtype=release",
            "-Dexamples=false",
            "-Dtests=false",
            "-Dutils=false",
            build_dir.to_str().unwrap(),
            src.to_str().unwrap(),
        ]));
    }
    run(Command::new("meson").args(["compile", "-C", build_dir.to_str().unwrap()]));

    println!(
        "cargo:rustc-link-search=native={}",
        build_dir.join("src").display()
    );
    println!("cargo:rustc-link-lib=static=tinypipewire");
    pkg_config::Config::new()
        .atleast_version("0.3.50")
        .probe("libpipewire-0.3")
        .expect("libpipewire-0.3 development files are required to build tinypipewire");

    println!("cargo:rerun-if-changed={}", src.join("src").display());
    println!("cargo:rerun-if-changed={}", src.join("include").display());

    // tpw_version.h is generated into the build directory, the rest ship in the tree.
    vec![src.join("include"), build_dir.join("include")]
}

/// The pinned C sources checked out as a submodule.
fn vendor_dir() -> PathBuf {
    PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .expect("the crate always sits inside the workspace")
        .join("vendor/tinypipewire")
}

fn run(cmd: &mut Command) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("failed to run {cmd:?}: {e}"));
    assert!(status.success(), "{cmd:?} failed with {status}");
}

fn write_bindings(include_paths: &[PathBuf]) {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("bindings.rs");
    let pregenerated = Path::new("src/bindings/pregenerated.rs");

    #[cfg(feature = "bindgen")]
    if env::var_os("DOCS_RS").is_none() {
        let mut builder = bindgen::Builder::default()
            .header("wrapper.h")
            // Installed headers sit on the system include path, where clang
            // drops comments unless asked to keep them.
            .clang_arg("-fretain-comments-from-system-headers")
            .allowlist_item("tpw_.*")
            .allowlist_item("TPW_.*")
            .derive_default(true)
            .derive_eq(true)
            .use_core()
            .ctypes_prefix("::core::ffi")
            .prepend_enum_name(false)
            .default_enum_style(bindgen::EnumVariation::Consts)
            .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));
        for path in include_paths {
            builder = builder.clang_arg(format!("-I{}", path.display()));
        }

        let bindings = builder.generate().expect("failed to generate bindings");
        bindings
            .write_to_file(&out)
            .expect("failed to write bindings");
        if env::var_os("TINYPIPEWIRE_SYS_UPDATE_BINDINGS").is_some() {
            bindings
                .write_to_file(pregenerated)
                .expect("failed to refresh the committed bindings");
        }
        return;
    }

    let _ = include_paths;
    std::fs::copy(pregenerated, &out).expect("failed to install the committed bindings");
}
