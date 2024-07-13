use anyhow::Result;

use git2::Repository;

fn main() -> Result<()> {
    let repo = Repository::open(".")?;

    let hash = repo.head()?.peel_to_commit()?.id();
    let hash_short = hash.to_string().chars().take(7).collect::<String>();

    println!("cargo:rustc-env=GIT_HASH={hash}");
    println!("cargo:rustc-env=GIT_HASH_SHORT={hash_short}");

    let tagname = repo
        .tag_names(None)?
        .iter()
        .filter_map(|optional_name| optional_name)
        .find(|name| {
            let Ok(reference) = repo.revparse_single(name) else {
                return false;
            };

            let Some(tag) = reference.as_commit() else {
                return false;
            };

            tag.id() == hash
        })
        .map(|name| name.to_owned());

    if let Some(tagname) = tagname {
        println!("cargo:rustc-env=GIT_TAG={tagname}");
    }

    Ok(())
}
