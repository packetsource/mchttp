use std::error::Error;
use std::process::Command;

pub fn main() -> Result<(), Box<dyn Error>> {
    // git show -s --format="%ad %h %an <%ae> (%s)"
    let output = Command::new("git")
        .args(&["show", "-s", "--format=%ad %h %an <%ae> (%s)"])
        .output()
        .unwrap();
    let git_hash = String::from_utf8(output.stdout).unwrap();
    //    let git_hash = "ffoo";
    println!("cargo:rustc-env=GIT_COMMITID={git_hash}");

    Ok(())
}
