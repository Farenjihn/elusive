use std::path::{Path, PathBuf};

pub fn search_paths<P, S>(name: P, paths: &[S]) -> Option<PathBuf>
where
    P: AsRef<Path>,
    S: AsRef<Path>,
{
    let mut list = paths.iter().map(|path| path.as_ref().join(&name));
    list.find(|path| path.exists())
}
