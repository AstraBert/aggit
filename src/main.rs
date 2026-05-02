mod gitops;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::gitops::{
    add, cat_file, commit, config_author, diff, init, ls_files, status, switch_branch,
};

#[derive(Parser)]
struct CliArgs {
    #[command(subcommand)]
    cmd: Commands,
}
#[derive(Subcommand, Debug)]
enum Commands {
    /// Initialize a directory as an aggit repository.
    ///
    /// Creates an `.aggit` subfolder in the target directory.
    Init {
        /// Path to the directory to initialize
        path: String,
    },

    /// Configure the global commit author.
    ///
    /// Creates a `~/.aggit/author.toml` file with email and name of the configured author.
    Author {
        /// Author name
        #[arg(short, long)]
        name: String,
        /// Author email
        #[arg(short, long)]
        email: String,
    },

    /// Add one or more files to the current aggit index (stashing area).
    Add {
        /// Files to be added to the index
        files: Vec<String>,
    },

    /// Commit stashed files.
    ///
    /// Updates the current main ref to point to the newly created commit.
    Commit {
        /// Commit message
        #[arg(short, long)]
        message: String,
    },

    /// Get the current status of the aggit repository.
    ///
    /// Displays changed, added and removed files.
    Status {},

    ///Write the contents of (or info about) object with given SHA-1 prefix to
    ///stdout. If mode is 'commit', 'tree', or 'blob', print raw data bytes of
    ///object. If mode is 'size', print the size of the object. If mode is
    ///'type', print the type of the object. If mode is 'pretty', print a
    ///prettified version of the object.
    Cat {
        /// Modality with which to print the file content.
        ///
        /// Allowed values: 'commit', 'tree', 'blob', 'type', 'size', 'pretty'
        #[arg(short, long)]
        mode: String,
        /// SHA1 hash of the object whose content should be printed.
        #[arg(short, long)]
        sha1: String,
    },
    /// Print list of files in index (including mode, SHA-1, and stage number
    /// if "details" is True).
    Ls {
        /// Include details for files in the index
        #[arg(short, long, default_value_t = false)]
        details: bool,
    },

    /// Show diff of files changed (between index and working copy).
    Diff {},

    /// Switch to a different branch.
    ///
    /// If `--create/-c` is provided and the target branch does not exist, create it from the current branch and commit.
    Switch {
        name: String,
        #[arg(short, long, default_value_t = false)]
        create: bool,
    },
}

fn main() -> anyhow::Result<()> {
    let args = CliArgs::parse();
    match args.cmd {
        Commands::Init { path } => {
            init(PathBuf::from(&path))?;
        }
        Commands::Status {} => {
            status()?;
        }
        Commands::Author { name, email } => {
            config_author(name, email)?;
        }
        Commands::Cat { mode, sha1 } => {
            cat_file(&mode, &sha1)?;
        }
        Commands::Ls { details } => {
            ls_files(details)?;
        }
        Commands::Add { files } => {
            add(files)?;
        }
        Commands::Commit { message } => {
            commit(&message)?;
        }
        Commands::Diff {} => {
            diff()?;
        }
        Commands::Switch { name, create } => {
            switch_branch(&name, create)?;
        }
    }

    Ok(())
}
