extern crate gcc;

fn main() {
    let target = std::env::var("TARGET").unwrap();

    // On Windows MSVC, output .lib (COFF) instead of .a (GNU archive).
    // The gcc crate's compile() hardcodes .a extension, which causes
    // lib.exe to produce a GNU-format archive that link.exe cannot read.
    let lib_name = if target.contains("windows") && target.contains("msvc") {
        "liblmdb.lib"
    } else {
        "liblmdb.a"
    };

    let mut config = gcc::Config::new();
    config.file("mdb/libraries/liblmdb/mdb.c")
          .file("mdb/libraries/liblmdb/midl.c");
    config.opt_level(2);

    if target.contains("dragonfly") {
        config.flag("-DMDB_DSYNC=O_SYNC");
        config.flag("-DMDB_FDATASYNC=fsync");
    }

    config.compile(lib_name);
}
