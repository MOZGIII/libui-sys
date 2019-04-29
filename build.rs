use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    // Prepare source.
    if !Path::new("libui/.git").exists() {
        let _ = Command::new("git")
            .args(&["submodule", "update", "--init", "--recursive"])
            .status();
    }

    // Generate bindings.
    let bindings = bindgen::Builder::default()
        .header("libui/ui.h")
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");

    // Determine target properties.
    let target = env::var("TARGET").unwrap();
    let msvc = target.contains("msvc");
    let windows = target.contains("windows");
    let linux = target.contains("linux");

    // Prepare linking options.
    let static_linking = env::var_os("LIBUI_SYS_STATIC_BUILD").is_some()
        || env::var_os("CARGO_FEATURE_STATIC").is_some();

    // Build library.
    let build_path = out_path.join("build");
    run_meson("libui", &build_path, static_linking);

    // Link library.
    let build_out_path = build_path.join("meson-out");
    if msvc && static_linking {
        // See https://github.com/mesonbuild/meson/issues/1412
        // With MSVC Rust searches for "<name-without-lib>.lib", but meson
        // generates "<name-with-lib>.a". Make them play together.
        fs::copy(
            build_out_path.join("libui.a"),
            build_out_path.join("ui.lib"),
        )
        .unwrap();
    }
    println!(
        "cargo:rustc-link-search=native={}",
        build_out_path.to_str().unwrap()
    );
    println!(
        "cargo:rustc-link-lib={}={}",
        if static_linking { "static" } else { "dylib" },
        if msvc && !static_linking {
            "libui"
        } else {
            "ui"
        }
    );

    if static_linking {
        if windows {
            // TODO: extract this data from mesos.
            for dep in [
                "user32",
                "kernel32",
                "gdi32",
                "comctl32",
                "uxtheme",
                "msimg32",
                "comdlg32",
                "d2d1",
                "dwrite",
                "ole32",
                "oleaut32",
                "oleacc",
                "uuid",
                "windowscodecs",
            ]
            .iter()
            {
                println!("cargo:rustc-link-lib=dylib={}", dep);
            }
        }
        if linux {
            pkg_config::Config::new()
                .atleast_version("3.10.0")
                .probe("gtk+-3.0")
                .unwrap();
        }
    }

    // Embed manifests for shared library.
    if !static_linking {
        embed_resource::compile("shared_resources.rc");
    }
}

fn run_meson<L, D>(lib: L, dir: D, static_linking: bool)
where
    L: AsRef<OsStr>,
    D: AsRef<OsStr>,
{
    if !is_configured(dir.as_ref()) {
        run_command(
            lib,
            "meson",
            &[
                OsStr::new("."),
                dir.as_ref(),
                OsStr::new("--default-library"),
                OsStr::new(if static_linking { "static" } else { "shared" }),
                OsStr::new("--buildtype=release"),
            ],
        );
    }
    run_command(dir, "ninja", &[]);
}

fn run_command<D, N>(dir: D, name: N, args: &[&OsStr])
where
    D: AsRef<OsStr>,
    N: AsRef<OsStr>,
{
    let mut cmd = Command::new(name);
    cmd.current_dir(dir.as_ref());
    if args.len() > 0 {
        cmd.args(args);
    }
    let out = match cmd.output() {
        Ok(v) => v,
        Err(err) => panic!("unable to invoke {:?}: {}", cmd, err),
    };
    if !out.status.success() {
        // This does not work great on Windows with non-ascii output,
        // but for now it"s good enough.
        let errtext = String::from_utf8_lossy(&out.stderr);
        let outtext = String::from_utf8_lossy(&out.stdout);
        panic!("{:?} invocation failed:\n{}\n{}", cmd, outtext, errtext);
    }
}

fn is_configured<S>(dir: S) -> bool
where
    S: AsRef<OsStr>,
{
    let mut path = PathBuf::from(dir.as_ref());
    path.push("build.ninja");
    return path.exists();
}
