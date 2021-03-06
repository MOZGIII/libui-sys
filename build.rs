use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    // Generate bindings.
    let bindings = bindgen::Builder::default()
        .header("libui/ui.h")
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from(env::var_os("OUT_DIR").expect("Unable to read OUT_DIR env var"));
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

    if msvc && !static_linking {
        // Detect windres executable location and populate the env var for meson.
        detect_windres_msvc();
    }

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
        .expect("Unable to copy libui.a to ui.lib");
    }
    if linux && !static_linking {
        // Symlink the shared library from versioned name to a non-versioned
        // name to len liner (ld) find it.
        if let Err(err) = fs::remove_file(build_out_path.join("libui.so")) {
            if err.kind() != io::ErrorKind::NotFound {
                panic!("Unable to remove libui.so: {:?}", err)
            }
        }
        symlink_file(
            build_out_path.join("libui.so.0"),
            build_out_path.join("libui.so"),
        )
        .expect("Unable to symlink libui.so.0 to libui.so");
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
            // TODO: extract this data from meson.
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
                .expect("Unable to perform pkg-config search");
        }
    }

    // Embed manifests for shared library.
    if !static_linking {
        embed_resource::compile("shared_resources.rc");
    }
}

#[cfg(target_os = "windows")]
fn detect_windres_msvc() {
    if std::env::var_os("DO_NOT_DETECT_WINDRES") != None {
        return;
    }

    if std::env::var_os("WINDRES") == None {
        let sdk_info = find_winsdk::SdkInfo::find(find_winsdk::SdkVersion::Any)
            .expect("Error: finding Win SDK errored out");

        if let Some(sdk_info) = sdk_info {
            let sdk_folder = sdk_info.installation_folder();

            let windres_path = match env::var("CARGO_CFG_TARGET_ARCH") {
                Ok(ref arch) if arch == "x86_64" => sdk_folder.join("bin/x64/rc.exe"),
                Ok(ref arch) if arch == "x86" => sdk_folder.join("bin/x86/rc.exe"),
                Ok(other) => panic!{"Unsupported target architecture: {}", other},
                Err(e) => panic!{"Error getting target arch {}", e}
            };

            // double-quote path to escape spaces
            std::env::set_var("WINDRES", format!(r#""{}""#, windres_path.display()))
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn detect_windres_msvc() {
    // noop
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
                OsStr::new("--backend=ninja"),
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

#[cfg(windows)]
fn symlink_file<P: AsRef<Path>, Q: AsRef<Path>>(src: P, dst: Q) -> io::Result<()> {
    std::os::windows::fs::symlink_file(src, dst)
}

#[cfg(not(windows))]
fn symlink_file<P: AsRef<Path>, Q: AsRef<Path>>(src: P, dst: Q) -> io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}
