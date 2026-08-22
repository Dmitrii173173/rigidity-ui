//! The project file.
//!
//! Stage two had two clouds and no need to remember anything: a session
//! was a pair of files opened by hand and a pose that lived as long as the
//! window. A survey is forty scans, each somewhere, and nobody re-places
//! forty scans because they closed a window. This module is the format
//! that survives that.
//!
//! It is the viewer's format and not the core's, deliberately. What it
//! holds — which files are in the scene, where each sits, which two are
//! playing source and target — are questions about a screen. The core has
//! no opinion about any of them and gains nothing from learning one, and
//! the alternative was `serde` in `rigidity-core`, bought for this.
//!
//! Text, because a project that cannot be read, diffed or repaired in an
//! editor is a project that has to be trusted rather than checked, and the
//! whole point of the application is not trusting numbers you cannot see.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use rigidity_core::lie::{Se3, So3};
use rigidity_core::nalgebra as na;

/// The only version this code writes, and the only one it reads.
///
/// A reader that guesses at a version it does not know is a reader that
/// silently drops whatever was added — and what would be dropped here is a
/// pose, which is the one thing the file exists to carry.
const VERSION: u32 = 1;

/// The first line of every project file.
const MAGIC: &str = "rigidity-project";

/// The extension the save dialog offers.
pub(crate) const EXTENSION: &str = "rgp";

/// Which part a scan is playing, if any.
///
/// Roles are a property of the session and not of the scan, but they are
/// written all the same: reopening a project and having to say again which
/// two of forty clouds you were registering is the kind of small forgetting
/// that makes a format feel unfinished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Role {
    /// Neither.
    #[default]
    Idle,
    /// The cloud everything else is registered to.
    Target,
    /// The cloud being moved.
    Source,
}

impl Role {
    fn word(self) -> &'static str {
        match self {
            Self::Idle => "none",
            Self::Target => "target",
            Self::Source => "source",
        }
    }

    fn parse(word: &str) -> Option<Self> {
        match word {
            "none" => Some(Self::Idle),
            "target" => Some(Self::Target),
            "source" => Some(Self::Source),
            _ => None,
        }
    }
}

/// One scan: a file, and where it sits.
#[derive(Debug, Clone)]
pub(crate) struct Scan {
    /// Absolute once read, whatever the file said.
    pub(crate) path: PathBuf,
    /// The motion carrying the file's own coordinates into the survey.
    pub(crate) pose: Se3,
    pub(crate) visible: bool,
    pub(crate) role: Role,
}

/// A scene, saved.
#[derive(Debug, Clone, Default)]
pub(crate) struct Project {
    pub(crate) scans: Vec<Scan>,
}

/// What a project file can be wrong about.
#[derive(Debug)]
pub(crate) enum Error {
    /// The file could not be read or written.
    Io(io::Error),
    /// The first line was not this format's.
    NotAProject,
    /// The format is from a later version of the application.
    Version(u32),
    /// A line was malformed, and which.
    Line {
        /// One-based, as an editor counts.
        number: usize,
        /// What was wrong with it.
        problem: &'static str,
    },
    /// Two scans claimed the same role.
    ///
    /// Not repaired by keeping the first: a file that says two things
    /// about which cloud is the target does not have a defensible answer
    /// hiding in it, and choosing one silently would put the survey in a
    /// state its own file does not describe.
    Duplicate(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::NotAProject => write!(f, "not a {MAGIC} file"),
            Self::Version(found) => {
                write!(f, "project version {found}, and this build reads {VERSION}")
            }
            Self::Line { number, problem } => write!(f, "line {number}: {problem}"),
            Self::Duplicate(role) => write!(f, "two scans claim to be the {role}"),
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl Project {
    /// Writes the project, with paths relative to it where they can be.
    ///
    /// Relative on purpose: a survey is a directory that gets copied to
    /// another machine, and absolute paths turn every one of those copies
    /// into a file full of dangling references. A scan that lives outside
    /// the project's own directory keeps its absolute path, because the
    /// alternative is a chain of `../..` that breaks the moment either end
    /// moves.
    pub(crate) fn write(&self, path: &Path) -> Result<(), Error> {
        let base = path.parent().unwrap_or(Path::new(""));
        std::fs::write(path, self.render(base))?;
        Ok(())
    }

    /// Reads a project, resolving relative paths against its own directory.
    pub(crate) fn read(path: &Path) -> Result<Self, Error> {
        let base = path.parent().unwrap_or(Path::new("")).to_path_buf();
        Self::parse(&std::fs::read_to_string(path)?, &base)
    }

    /// The file's text, without touching a disk.
    ///
    /// Separate from [`Project::write`] so that the round trip can be
    /// tested for what it actually guarantees — that the numbers survive —
    /// rather than for whether a temporary directory was writable.
    pub(crate) fn render(&self, base: &Path) -> String {
        let mut out = format!("{MAGIC} {VERSION}\n");
        for scan in &self.scans {
            let path = scan
                .path
                .strip_prefix(base)
                .unwrap_or(scan.path.as_path())
                .to_string_lossy()
                .into_owned();
            let rotation = scan.pose.rotation().matrix();
            let translation = scan.pose.translation();
            out.push_str("\nscan  ");
            out.push_str(&path);
            out.push_str("\npose ");
            // Row-major, then the translation. Twelve numbers rather than
            // the six of a twist, and this is the gate rather than a
            // preference: `Se3` stores a rotation *matrix*, so writing
            // `log` and reading `exp` would round-trip through two
            // transcendental functions and come back near the pose rather
            // than at it. A project that drifts every time it is saved
            // cannot be used to measure drift.
            for row in 0..3 {
                for column in 0..3 {
                    out.push(' ');
                    out.push_str(&number(rotation[(row, column)]));
                }
            }
            for axis in 0..3 {
                out.push(' ');
                out.push_str(&number(translation[axis]));
            }
            out.push_str("\nrole  ");
            out.push_str(scan.role.word());
            out.push_str("\nshown ");
            out.push_str(if scan.visible { "yes" } else { "no" });
            out.push('\n');
        }
        out
    }

    /// Parses the file's text, resolving relative paths against `base`.
    pub(crate) fn parse(text: &str, base: &Path) -> Result<Self, Error> {
        let mut lines = text.lines().enumerate();
        let (_, first) = lines.next().ok_or(Error::NotAProject)?;
        let mut header = first.split_whitespace();
        if header.next() != Some(MAGIC) {
            return Err(Error::NotAProject);
        }
        match header.next().and_then(|word| word.parse::<u32>().ok()) {
            Some(VERSION) => {}
            Some(found) => return Err(Error::Version(found)),
            None => return Err(Error::NotAProject),
        }

        let mut scans: Vec<Scan> = Vec::new();
        for (index, line) in lines {
            let number = index + 1;
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            let rest = rest.trim();

            if key == "scan" {
                if rest.is_empty() {
                    return Err(Error::Line {
                        number,
                        problem: "a scan needs a path",
                    });
                }
                let path = PathBuf::from(rest);
                scans.push(Scan {
                    path: if path.is_absolute() {
                        path
                    } else {
                        base.join(path)
                    },
                    pose: Se3::identity(),
                    visible: true,
                    role: Role::Idle,
                });
                continue;
            }

            // Every other key describes the scan above it. A file that
            // opens with one has lost its `scan` line somewhere, and
            // guessing which scan was meant is how a pose ends up on the
            // wrong cloud.
            let Some(scan) = scans.last_mut() else {
                return Err(Error::Line {
                    number,
                    problem: "this belongs to a scan, and no scan has been named yet",
                });
            };

            match key {
                "pose" => scan.pose = pose(rest, number)?,
                "role" => {
                    scan.role = Role::parse(rest).ok_or(Error::Line {
                        number,
                        problem: "a role is none, target or source",
                    })?
                }
                "shown" => {
                    scan.visible = match rest {
                        "yes" => true,
                        "no" => false,
                        _ => {
                            return Err(Error::Line {
                                number,
                                problem: "shown is yes or no",
                            });
                        }
                    }
                }
                _ => {
                    return Err(Error::Line {
                        number,
                        problem: "unknown key",
                    });
                }
            }
        }

        for (role, word) in [(Role::Target, "target"), (Role::Source, "source")] {
            if scans.iter().filter(|scan| scan.role == role).count() > 1 {
                return Err(Error::Duplicate(word));
            }
        }

        Ok(Self { scans })
    }
}

/// One `f64`, written so that reading it back gives the same bits.
///
/// Rust's own float formatting is the shortest decimal that parses back to
/// the identical value, which is exactly the guarantee this file needs and
/// is why no precision is specified here: `{:.17}` would be longer, no more
/// exact, and would suggest a decision had been made about how much of the
/// number to keep.
fn number(value: f64) -> String {
    format!("{value:?}")
}

/// Twelve numbers back into a pose.
fn pose(rest: &str, number: usize) -> Result<Se3, Error> {
    let mut values = [0.0f64; 12];
    let mut found = 0;
    for word in rest.split_whitespace() {
        if found == values.len() {
            return Err(Error::Line {
                number,
                problem: "a pose is twelve numbers: nine of rotation, three of translation",
            });
        }
        values[found] = word.parse().map_err(|_| Error::Line {
            number,
            problem: "a pose is twelve numbers: nine of rotation, three of translation",
        })?;
        found += 1;
    }
    if found != values.len() {
        return Err(Error::Line {
            number,
            problem: "a pose is twelve numbers: nine of rotation, three of translation",
        });
    }
    // Row-major in, column-major storage: `Matrix3::new` takes rows, which
    // is the order they were written in.
    let rotation = na::Matrix3::new(
        values[0], values[1], values[2], values[3], values[4], values[5], values[6], values[7],
        values[8],
    );
    // Unchecked, and it has to be. The matrix came back from decimal, so
    // asking it to be orthogonal to the last bit would reject poses this
    // very function wrote. Its orthogonality is the writer's business.
    Ok(Se3::from_parts(
        So3::from_matrix_unchecked(rotation),
        na::Vector3::new(values[9], values[10], values[11]),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Twenty scans, each somewhere different, at survey coordinates.
    ///
    /// The rotations come from `exp` rather than from typed-out matrices so
    /// that their entries are the irrational-looking `f64`s a real solver
    /// produces — a round trip that only preserves 0, 1 and 0.5 has not
    /// been tested. The translations are at UTM scale, where the gap
    /// between adjacent `f64`s is around a nanometre and the gap between
    /// adjacent `f32`s is a quarter of a metre.
    fn survey() -> Project {
        (0..20)
            .map(|index| {
                let turn = index as f64;
                let pose = Se3::from_parts(
                    So3::exp(&na::Vector3::new(
                        turn * 0.017_453_292_519_943_295,
                        turn * -0.031_415_926_535_897_93,
                        turn * 0.007_853_981_633_974_483,
                    )),
                    na::Vector3::new(
                        512_345.678_9 + turn * 1.234_567_891_011_121,
                        4_123_456.789_1 - turn * 0.987_654_321_098_765,
                        231.5 + turn * 0.000_123_456_789,
                    ),
                );
                Scan {
                    path: PathBuf::from(format!("scans/station-{index:02}.laz")),
                    pose,
                    visible: index % 3 != 0,
                    role: match index {
                        0 => Role::Target,
                        1 => Role::Source,
                        _ => Role::Idle,
                    },
                }
            })
            .collect::<Vec<_>>()
            .into()
    }

    impl From<Vec<Scan>> for Project {
        fn from(scans: Vec<Scan>) -> Self {
            Self { scans }
        }
    }

    fn bits(pose: &Se3) -> Vec<u64> {
        pose.rotation()
            .matrix()
            .iter()
            .chain(pose.translation().iter())
            .map(|value| value.to_bits())
            .collect()
    }

    /// The gate. Not "close enough": the same bits.
    ///
    /// A project is the thing a survey's drift is measured against, so a
    /// project that moves a little every time it is written is a ruler that
    /// changes length when you put it down.
    #[test]
    fn twenty_scans_reopen_at_exactly_the_same_poses() {
        let base = Path::new("/surveys/hauptgebaude");
        let before = survey();
        let text = before.render(base);
        let after = Project::parse(&text, base).expect("the file we just wrote should parse");

        assert_eq!(after.scans.len(), 20, "a scan went missing");
        for (index, (before, after)) in before.scans.iter().zip(&after.scans).enumerate() {
            assert_eq!(
                bits(&before.pose),
                bits(&after.pose),
                "scan {index} came back at a different pose"
            );
        }
    }

    /// And again, so that a project reopened twice is not a project drifting
    /// twice as slowly.
    #[test]
    fn a_second_round_trip_changes_nothing() {
        let base = Path::new("/surveys/hauptgebaude");
        let once = Project::parse(&survey().render(base), base).unwrap();
        let text = once.render(base);
        let twice = Project::parse(&text, base).unwrap();
        assert_eq!(text, twice.render(base), "the text is not a fixed point");
    }

    #[test]
    fn the_rest_of_a_scan_survives_too() {
        let base = Path::new("/surveys/hauptgebaude");
        let after = Project::parse(&survey().render(base), base).unwrap();
        assert_eq!(after.scans[0].role, Role::Target);
        assert_eq!(after.scans[1].role, Role::Source);
        assert_eq!(after.scans[2].role, Role::Idle);
        assert!(!after.scans[0].visible, "index 0 was hidden");
        assert!(after.scans[1].visible);
    }

    /// A project moved to another machine still finds its scans.
    #[test]
    fn a_path_inside_the_project_is_written_relative() {
        let here = Path::new("/surveys/hauptgebaude");
        let text = survey().render(here);
        assert!(
            text.contains("scan  scans/station-00.laz"),
            "the path was not made relative:\n{text}"
        );

        let there = Path::new("/mnt/backup/hauptgebaude");
        let moved = Project::parse(&text, there).unwrap();
        assert_eq!(
            moved.scans[0].path,
            there.join("scans/station-00.laz"),
            "a relative path did not follow the project"
        );
    }

    /// And one that lives elsewhere is left alone.
    #[test]
    fn a_path_outside_the_project_stays_absolute() {
        let project = Project {
            scans: vec![Scan {
                path: PathBuf::from("/data/shared/reference.laz"),
                pose: Se3::identity(),
                visible: true,
                role: Role::Idle,
            }],
        };
        let base = Path::new("/surveys/hauptgebaude");
        let text = project.render(base);
        assert!(text.contains("scan  /data/shared/reference.laz"), "{text}");
        assert_eq!(
            Project::parse(&text, Path::new("/somewhere/else"))
                .unwrap()
                .scans[0]
                .path,
            PathBuf::from("/data/shared/reference.laz")
        );
    }

    #[test]
    fn a_path_with_a_space_in_it_survives() {
        let project = Project {
            scans: vec![Scan {
                path: PathBuf::from("/point cloud head/a scan.laz"),
                pose: Se3::identity(),
                visible: true,
                role: Role::Idle,
            }],
        };
        let base = Path::new("/elsewhere");
        let after = Project::parse(&project.render(base), base).unwrap();
        assert_eq!(after.scans[0].path, project.scans[0].path);
    }

    #[test]
    fn a_later_version_is_refused_rather_than_guessed_at() {
        let text = format!("{MAGIC} {}\n", VERSION + 1);
        assert!(matches!(
            Project::parse(&text, Path::new("")),
            Err(Error::Version(_))
        ));
    }

    #[test]
    fn something_else_entirely_is_not_a_project() {
        for text in ["", "ply\nformat ascii 1.0\n", "rigidity-project\n"] {
            assert!(
                matches!(Project::parse(text, Path::new("")), Err(Error::NotAProject)),
                "{text:?} was accepted"
            );
        }
    }

    #[test]
    fn a_malformed_line_says_which_one() {
        let text = format!("{MAGIC} {VERSION}\nscan a.ply\npose 1 2 3\n");
        match Project::parse(&text, Path::new("")) {
            Err(Error::Line { number, .. }) => assert_eq!(number, 3),
            other => panic!("expected a line error, got {other:?}"),
        }
    }

    #[test]
    fn a_key_before_any_scan_is_an_error() {
        let text = format!("{MAGIC} {VERSION}\nrole target\n");
        assert!(matches!(
            Project::parse(&text, Path::new("")),
            Err(Error::Line { number: 2, .. })
        ));
    }

    #[test]
    fn an_unknown_key_is_an_error_rather_than_a_shrug() {
        let text = format!("{MAGIC} {VERSION}\nscan a.ply\ncolour red\n");
        assert!(matches!(
            Project::parse(&text, Path::new("")),
            Err(Error::Line { number: 3, .. })
        ));
    }

    #[test]
    fn two_targets_is_an_error() {
        let text = format!("{MAGIC} {VERSION}\nscan a.ply\nrole target\nscan b.ply\nrole target\n");
        assert!(matches!(
            Project::parse(&text, Path::new("")),
            Err(Error::Duplicate("target"))
        ));
    }

    /// The gate again, through a real file rather than a string.
    ///
    /// `render` and `parse` are where the numbers are at risk, and the two
    /// tests above cover them — but a project is a thing on a disk, and
    /// the path handling that only [`Project::write`] and [`Project::read`]
    /// perform is exactly where a pose could end up attached to the wrong
    /// scan.
    #[test]
    fn twenty_scans_survive_a_round_trip_through_the_filesystem() {
        let directory = std::env::temp_dir().join("rigidity-ui-project-round-trip");
        std::fs::create_dir_all(&directory).expect("a temporary directory");
        let file = directory.join(format!("survey.{EXTENSION}"));

        // Rooted in the directory the project will live in, so the paths
        // are the relative ones a real project writes.
        let mut before = survey();
        for scan in &mut before.scans {
            scan.path = directory.join(&scan.path);
        }

        before.write(&file).expect("the project should write");
        let after = Project::read(&file).expect("the project should read back");

        assert_eq!(after.scans.len(), before.scans.len());
        for (index, (before, after)) in before.scans.iter().zip(&after.scans).enumerate() {
            assert_eq!(before.path, after.path, "scan {index} lost its file");
            assert_eq!(
                bits(&before.pose),
                bits(&after.pose),
                "scan {index} came back at a different pose"
            );
            assert_eq!(before.visible, after.visible, "scan {index}");
            assert_eq!(before.role, after.role, "scan {index}");
        }

        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn comments_and_blank_lines_are_allowed() {
        let text = format!("{MAGIC} {VERSION}\n\n# the north wing\nscan a.ply\n\nshown no\n");
        let project = Project::parse(&text, Path::new("")).unwrap();
        assert_eq!(project.scans.len(), 1);
        assert!(!project.scans[0].visible);
    }
}
