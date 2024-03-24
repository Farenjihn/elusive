//! Systemd unit discovery and management.
//!
//! This module is helpful to get dependencies of a unit file, required binaries
//! executed by services and installation paths for symlink creation.

use crate::search::search_paths;

use pest::Parser;
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::{fs, io};

use self::parser::{Rule, UnitParser};

const UNIT_SEARCH_PATHS: &[&str] = &[
    "/usr/lib/systemd/system/",
    "/etc/systemd/system/",
    "/etc/systemd/system.control/",
    "/etc/systemd/system.attached/",
];

const UNIT_INSTALL_PATHS: &[&str] = &[
    "/usr/lib/systemd/system/initrd.target.wants/",
    "/usr/lib/systemd/system/initrd-root-device.target.wants/",
    "/usr/lib/systemd/system/initrd-root-fs.target.wants/",
    "/usr/lib/systemd/system/sysinit.target.wants/",
];

const SOCKET_INSTALL_PATHS: &[&str] = &["/usr/lib/systemd/system/sockets.target.wants/"];

mod parser {
    use pest_derive::Parser;

    #[derive(Parser)]
    #[grammar = "systemd.pest"]
    pub struct UnitParser;
}

/// Custom error type for unit file processing.
#[derive(thiserror::Error, Debug)]
pub enum UnitError {
    #[error("i/o error: {0}")]
    InputOutput(io::Error),
    #[error("failed to parse unit: {0}")]
    Parse(Box<pest::error::Error<Rule>>),
    #[error("could not find systemd unit: {0:?}")]
    UnitNotFound(OsString),
}

impl From<io::Error> for UnitError {
    fn from(err: io::Error) -> Self {
        Self::InputOutput(err)
    }
}

impl From<pest::error::Error<Rule>> for UnitError {
    fn from(err: pest::error::Error<Rule>) -> Self {
        Self::Parse(Box::new(err))
    }
}

/// Type representing a systemd unit (service, socket, path, target, ...).
#[derive(Debug)]
pub struct Unit {
    /// The path of the unit in the filesystem.
    pub path: PathBuf,
    /// The raw bytes of the unit file.
    pub data: Vec<u8>,
    /// The binaries required by this unit (non-empty if service, empty otherwise).
    pub binaries: Vec<String>,
    /// The dependencies of this unit file (Requires=).
    pub dependencies: Vec<String>,
    /// The path where a symlink should be created for installation.
    pub install_path: Option<PathBuf>,
}

impl Unit {
    /// Search and parse a unit file with the given name.
    pub fn from_name<T>(name: T) -> Result<Self, UnitError>
    where
        T: AsRef<str>,
    {
        let name = name.as_ref();
        let path = Self::find_unit(name)?;
        let data = fs::read_to_string(&path)?;

        let unit = UnitParser::parse(Rule::unit, &data)?
            .next()
            .expect("parsing succeeded");

        let mut properties: BTreeMap<&str, BTreeMap<&str, Vec<&str>>> = BTreeMap::new();
        let mut current_section = "";

        for pair in unit.into_inner() {
            match pair.as_rule() {
                Rule::section => {
                    let mut rules = pair.into_inner();
                    current_section = rules.next().unwrap().as_str();
                }
                Rule::property => {
                    let mut rules = pair.into_inner();

                    let name: &str = rules.next().unwrap().as_str();
                    let value: &str = rules.next().unwrap().as_str();

                    let section = properties.entry(current_section).or_default();
                    let list = section.entry(name).or_default();
                    list.push(value);
                }
                Rule::EOI => (),
                other => unreachable!("{other:?}"),
            }
        }

        let mut binaries = Vec::new();
        if let Some(service) = properties.get("Service") {
            // TODO: ExecStartPre and ExecStartPost non-optional
            let commands = service
                .get("ExecStart")
                .expect("ExecStart property is declared");

            for command in commands {
                binaries.push(cmd_exec_path(command));
            }
        }

        let mut dependencies = Vec::new();
        let section = properties.get("Unit").expect("Unit section is declared");

        if let Some(required) = section.get("Requires") {
            let iter = required[0]
                .split(' ')
                .filter(|dep| !dep.ends_with(".slice"))
                .map(String::from);

            dependencies.extend(iter);
        }

        let static_paths = match path.extension().and_then(OsStr::to_str) {
            Some("path" | "service" | "target") => UNIT_INSTALL_PATHS,
            Some("socket") => SOCKET_INSTALL_PATHS,
            _ => &[],
        };

        let install_path = static_paths
            .iter()
            .map(|path| Path::new(path).join(name))
            .find(|path| path.exists());

        Ok(Unit {
            path,
            data: data.into_bytes(),
            binaries,
            dependencies,
            install_path,
        })
    }

    /// Search for a unit with the given name (e.g. `systemd-journald.service`) and
    /// return its path if it exists.
    pub fn find_unit<P>(name: P) -> Result<PathBuf, UnitError>
    where
        P: AsRef<Path>,
    {
        search_paths(&name, UNIT_SEARCH_PATHS)
            .ok_or_else(|| UnitError::UnitNotFound(name.as_ref().into()))
    }
}

fn cmd_exec_path<T>(command: T) -> String
where
    T: AsRef<str>,
{
    command
        .as_ref()
        .split(' ')
        .next()
        .expect("command is a space separated string")
        .trim_start_matches(['@', '!', ':', '+', '-'])
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parser() {
        let units = ["kmod-static-nodes.service", "systemd-journald.service"];

        for unit in units {
            let path = Unit::find_unit(unit).unwrap();
            let data = fs::read_to_string(&path).unwrap();
            UnitParser::parse(Rule::unit, &data).unwrap();
        }
    }
}
