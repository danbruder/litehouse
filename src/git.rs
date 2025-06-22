use anyhow::Result;
use gix::Repository;

pub struct GitPullResult {
    pub commit: String,
}

pub fn pull(remote_name: &str, branch: &str, directory: &str) -> Result<GitPullResult> {
    let repo = Repository::open(directory)?;

    let remote = repo
        .find_remote(remote_name)
        .or_else(|_| repo.remote_at(format!("refs/remotes/{}", remote_name).as_str()))?;

    let connection = remote
        .connect(gix::remote::Direction::Fetch)?
        .prepare_fetch(
            &mut gix::progress::Discard,
            gix::remote::ref_map::Options::default(),
        )?;

    let outcome =
        connection.receive(&mut gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)?;

    let refs_to_update = outcome
        .ref_map
        .mappings
        .iter()
        .filter(|mapping| {
            mapping.remote.as_name().map_or(false, |name| {
                name.as_bstr()
                    .starts_with(format!("refs/heads/{}", branch).as_bytes())
            })
        })
        .collect::<Vec<_>>();

    for mapping in refs_to_update {
        if let Some(local_name) = &mapping.local {
            if let Some(remote_id) = mapping.remote.as_id() {
                repo.refs
                    .transaction()
                    .prepare()?
                    .create(
                        local_name,
                        *remote_id,
                        gix::refs::transaction::PreviousValue::Any,
                        "pull",
                    )?
                    .commit()?;
            }
        }
    }

    // Get the latest commit hash for the branch
    let branch_ref = repo.find_reference(format!("refs/heads/{}", branch))?;

    Ok(GitPullResult {
        commit: branch_ref.object_id().to_string(),
    })
}
