use log::trace;

use crate::{
    config::Config,
    fs::{expand, get_included_paths_list},
    fzf::execute_fzf_command,
    Error,
};

pub(crate) fn select_from_list(
    list: &str,
    header: &'static str,
    args: &[&str],
) -> Result<String, crate::Error> {
    let result = execute_fzf_command(args.iter().chain(&["--header", header]).cloned(), list)?;
    if result.is_empty() {
        trace!("Empty pick");
        Err(crate::Error::EmptyPick())
    } else {
        trace!("Pick: {}", result);
        Ok(result)
    }
}

pub(crate) fn pick_project(config: &Config, header: &'static str) -> Result<String, Error> {
    // get dirs' paths
    let dirs = {
        let mut list = vec![];
        for include_entry in config.include.iter() {
            for path in &include_entry.paths {
                let expanded_path = expand(path)?;
                if include_entry.include_intermediate_paths {
                    list.push(expanded_path.clone());
                }
                get_included_paths_list(&expanded_path, 0, &mut list, include_entry, config)?;
            }
        }
        list.join("\n")
    };

    // pick one from list with fzf
    let pick = select_from_list(
        &dirs,
        header,
        &[
            "--layout",
            "reverse",
            "--preview",
            "tree -C '{}'",
            "--preview-window",
            "right:nohidden",
        ],
    )?
    .trim_end()
    .to_owned();
    Ok(pick)
}
