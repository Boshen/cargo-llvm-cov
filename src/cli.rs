use std::{ffi::OsString, mem, str::FromStr};

use anyhow::{bail, format_err, Error, Result};
use camino::{Utf8Path, Utf8PathBuf};
use lexopt::{
    Arg::{Long, Short, Value},
    ValueExt,
};

use crate::{
    env,
    term::{self, Coloring},
};

// TODO: add --config option and passthrough to cargo-config: https://github.com/rust-lang/cargo/pull/10755/

#[derive(Debug)]
pub(crate) struct Args {
    pub(crate) subcommand: Subcommand,

    cov: LlvmCovOptions,
    pub(crate) show_env: ShowEnvOptions,

    // https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/instrument-coverage.html#including-doc-tests
    /// Including doc tests (unstable)
    ///
    /// This flag is unstable.
    /// See <https://github.com/taiki-e/cargo-llvm-cov/issues/2> for more.
    pub(crate) doctests: bool,

    // =========================================================================
    // `cargo test` options
    // https://doc.rust-lang.org/nightly/cargo/commands/cargo-test.html
    /// Generate coverage report without running tests
    pub(crate) no_run: bool,
    // /// Run all tests regardless of failure
    // pub(crate) no_fail_fast: bool,
    /// Run all tests regardless of failure and generate report
    ///
    /// If tests failed but report generation succeeded, exit with a status of 0.
    pub(crate) ignore_run_fail: bool,
    // /// Display one character per test instead of one line
    // pub(crate) quiet: bool,
    /// Test only this package's library unit tests
    pub(crate) lib: bool,
    /// Test only the specified binary
    pub(crate) bin: Vec<String>,
    /// Test all binaries
    pub(crate) bins: bool,
    /// Test only the specified example
    pub(crate) example: Vec<String>,
    /// Test all examples
    pub(crate) examples: bool,
    /// Test only the specified test target
    pub(crate) test: Vec<String>,
    /// Test all tests
    pub(crate) tests: bool,
    /// Test only the specified bench target
    pub(crate) bench: Vec<String>,
    /// Test all benches
    pub(crate) benches: bool,
    /// Test all targets
    pub(crate) all_targets: bool,
    /// Test only this library's documentation (unstable)
    ///
    /// This flag is unstable because it automatically enables --doctests flag.
    /// See <https://github.com/taiki-e/cargo-llvm-cov/issues/2> for more.
    pub(crate) doc: bool,
    // /// Package to run tests for
    // pub(crate) package: Vec<String>,
    /// Test all packages in the workspace
    pub(crate) workspace: bool,
    /// Exclude packages from both the test and report
    pub(crate) exclude: Vec<String>,
    /// Exclude packages from the test (but not from the report)
    pub(crate) exclude_from_test: Vec<String>,
    /// Exclude packages from the report (but not from the test)
    pub(crate) exclude_from_report: Vec<String>,

    pub(crate) build: BuildOptions,

    pub(crate) manifest: ManifestOptions,

    pub(crate) cargo_args: Vec<String>,
    /// Arguments for the test binary
    pub(crate) rest: Vec<String>,
}

impl Args {
    pub(crate) fn parse() -> Result<Self> {
        const SUBCMD: &str = "llvm-cov";

        // rustc/cargo args must be valid Unicode
        // https://github.com/rust-lang/rust/blob/1.62.0/compiler/rustc_driver/src/lib.rs#L1325-L1335
        fn handle_args(
            args: impl IntoIterator<Item = impl Into<OsString>>,
        ) -> impl Iterator<Item = Result<String>> {
            args.into_iter().enumerate().map(|(i, arg)| {
                arg.into()
                    .into_string()
                    .map_err(|arg| format_err!("argument {} is not valid Unicode: {arg:?}", i + 1))
            })
        }

        let mut raw_args = handle_args(env::args_os());
        raw_args.next(); // cargo
        match raw_args.next().transpose()? {
            Some(arg) if arg == SUBCMD => {}
            Some(arg) => bail!("expected subcommand '{SUBCMD}', found argument '{arg}'"),
            None => bail!("expected subcommand '{SUBCMD}'"),
        }
        let mut args = vec![];
        for arg in &mut raw_args {
            let arg = arg?;
            if arg == "--" {
                break;
            }
            args.push(arg);
        }
        let rest = raw_args.collect::<Result<Vec<_>>>()?;

        let mut cargo_args = vec![];
        let mut subcommand: Option<Subcommand> = None;

        let mut manifest_path = None;
        let mut color = None;

        let mut doctests = false;
        let mut no_run = false;
        let mut no_fail_fast = false;
        let mut ignore_run_fail = false;
        let mut lib = false;
        let mut bin = vec![];
        let mut bins = false;
        let mut example = vec![];
        let mut examples = false;
        let mut test = vec![];
        let mut tests = false;
        let mut bench = vec![];
        let mut benches = false;
        let mut all_targets = false;
        let mut doc = false;

        let mut package: Vec<String> = vec![];
        let mut workspace = false;
        let mut exclude = vec![];
        let mut exclude_from_test = vec![];
        let mut exclude_from_report = vec![];

        // llvm-cov options
        let mut json = false;
        let mut lcov = false;
        let mut text = false;
        let mut html = false;
        let mut open = false;
        let mut summary_only = false;
        let mut output_path = None;
        let mut output_dir = None;
        let mut failure_mode = None;
        let mut ignore_filename_regex = None;
        let mut disable_default_ignore_filename_regex = false;
        let mut hide_instantiations = false;
        let mut no_cfg_coverage = false;
        let mut no_cfg_coverage_nightly = false;
        let mut no_report = false;
        let mut fail_under_lines = None;
        let mut fail_uncovered_lines = None;
        let mut fail_uncovered_regions = None;
        let mut fail_uncovered_functions = None;
        let mut show_missing_lines = false;
        let mut include_build_script = false;

        // build options
        let mut jobs = None;
        let mut release = false;
        let mut profile = None;
        let mut target = None;
        let mut coverage_target_only = false;
        let mut remap_path_prefix = false;
        let mut include_ffi = false;
        let mut verbose: usize = 0;
        let mut no_clean = false;

        // show-env options
        let mut export_prefix = false;

        let mut parser = lexopt::Parser::from_args(args);
        while let Some(arg) = parser.next()? {
            macro_rules! parse_opt {
                ($opt:ident[] $(,)?) => {{
                    $opt.push(parser.value()?.parse()?);
                }};
                ($opt:ident $(,)?) => {{
                    if $opt.is_some() {
                        multi_arg(&arg)?;
                    }
                    $opt = Some(parser.value()?.parse()?);
                }};
            }
            macro_rules! parse_opt_passthrough {
                ($opt:ident[] $(,)?) => {{
                    match arg {
                        Long(flag) => {
                            let flag = format!("--{}", flag);
                            if let Some(val) = parser.optional_value() {
                                $opt.push(val.parse()?);
                                cargo_args.push(format!("{}={}", flag, val.into_string().unwrap()));
                            } else {
                                let val = parser.value()?.into_string().unwrap();
                                $opt.push(val.parse()?);
                                cargo_args.push(flag);
                                cargo_args.push(val);
                            }
                        }
                        Short(flag) => {
                            if let Some(val) = parser.optional_value() {
                                $opt.push(val.parse()?);
                                cargo_args.push(format!("-{}{}", flag, val.into_string().unwrap()));
                            } else {
                                let val = parser.value()?.into_string().unwrap();
                                $opt.push(val.parse()?);
                                cargo_args.push(format!("-{}", flag));
                                cargo_args.push(val);
                            }
                        }
                        Value(_) => unreachable!(),
                    }
                }};
                ($opt:ident $(,)?) => {{
                    if $opt.is_some() {
                        multi_arg(&arg)?;
                    }
                    match arg {
                        Long(flag) => {
                            let flag = format!("--{}", flag);
                            if let Some(val) = parser.optional_value() {
                                $opt = Some(val.parse()?);
                                cargo_args.push(format!("{}={}", flag, val.into_string().unwrap()));
                            } else {
                                let val = parser.value()?.into_string().unwrap();
                                $opt = Some(val.parse()?);
                                cargo_args.push(flag);
                                cargo_args.push(val);
                            }
                        }
                        Short(flag) => {
                            if let Some(val) = parser.optional_value() {
                                $opt = Some(val.parse()?);
                                cargo_args.push(format!("-{}{}", flag, val.into_string().unwrap()));
                            } else {
                                let val = parser.value()?.into_string().unwrap();
                                $opt = Some(val.parse()?);
                                cargo_args.push(format!("-{}", flag));
                                cargo_args.push(val);
                            }
                        }
                        Value(_) => unreachable!(),
                    }
                }};
            }
            macro_rules! parse_flag {
                ($flag:ident $(,)?) => {
                    if mem::replace(&mut $flag, true) {
                        multi_arg(&arg)?;
                    }
                };
            }
            macro_rules! parse_flag_passthrough {
                ($flag:ident $(,)?) => {{
                    parse_flag!($flag);
                    passthrough!();
                }};
            }
            macro_rules! passthrough {
                () => {
                    match arg {
                        Long(flag) => {
                            let flag = format!("--{}", flag);
                            if let Some(val) = parser.optional_value() {
                                cargo_args.push(format!("{}={}", flag, val.parse::<String>()?));
                            } else {
                                cargo_args.push(flag);
                            }
                        }
                        Short(flag) => {
                            if let Some(val) = parser.optional_value() {
                                cargo_args.push(format!("-{}{}", flag, val.parse::<String>()?));
                            } else {
                                cargo_args.push(format!("-{}", flag));
                            }
                        }
                        Value(_) => unreachable!(),
                    }
                };
            }

            match arg {
                Long("color") => parse_opt_passthrough!(color),
                Long("manifest-path") => parse_opt!(manifest_path),

                Long("doctests") => parse_flag!(doctests),
                Long("ignore-run-fail") => parse_flag!(ignore_run_fail),
                Long("no-run") => parse_flag!(no_run),
                Long("no-fail-fast") => parse_flag_passthrough!(no_fail_fast),

                Long("lib") => parse_flag_passthrough!(lib),
                Long("bin") => parse_opt_passthrough!(bin[]),
                Long("bins") => parse_flag_passthrough!(bins),
                Long("example") => parse_opt_passthrough!(example[]),
                Long("examples") => parse_flag_passthrough!(examples),
                Long("test") => parse_opt_passthrough!(test[]),
                Long("tests") => parse_flag_passthrough!(tests),
                Long("bench") => parse_opt_passthrough!(bench[]),
                Long("benches") => parse_flag_passthrough!(benches),
                Long("all-targets") => parse_flag_passthrough!(all_targets),
                Long("doc") => parse_flag_passthrough!(doc),

                Short('p') | Long("package") => parse_opt_passthrough!(package[]),
                Long("workspace" | "all") => parse_flag_passthrough!(workspace),
                Long("exclude") => parse_opt_passthrough!(exclude[]),
                Long("exclude-from-test") => parse_opt!(exclude_from_test[]),
                Long("exclude-from-report") => parse_opt!(exclude_from_report[]),

                // build options
                Short('j') | Long("jobs") => parse_opt_passthrough!(jobs),
                Short('r') | Long("release") => parse_flag_passthrough!(release),
                Long("profile") => parse_opt_passthrough!(profile),
                Long("target") => parse_opt_passthrough!(target),
                Long("coverage-target-only") => parse_flag!(coverage_target_only),
                Long("remap-path-prefix") => parse_flag!(remap_path_prefix),
                Long("include-ffi") => parse_flag!(include_ffi),
                Short('v') | Long("verbose") => verbose += 1,
                Long("no-clean") => parse_flag!(no_clean),

                // llvm-cov options
                Long("json") => parse_flag!(json),
                Long("lcov") => parse_flag!(lcov),
                Long("text") => parse_flag!(text),
                Long("html") => parse_flag!(html),
                Long("open") => parse_flag!(open),
                Long("summary-only") => parse_flag!(summary_only),
                Long("output-path") => parse_opt!(output_path),
                Long("output-dir") => parse_opt!(output_dir),
                Long("failure-mode") => parse_opt!(failure_mode),
                Long("ignore-filename-regex") => parse_opt!(ignore_filename_regex),
                Long("disable-default-ignore-filename-regex") => {
                    parse_flag!(disable_default_ignore_filename_regex);
                }
                Long("hide-instantiations") => parse_flag!(hide_instantiations),
                Long("no-cfg-coverage") => parse_flag!(no_cfg_coverage),
                Long("no-cfg-coverage-nightly") => parse_flag!(no_cfg_coverage_nightly),
                Long("no-report") => parse_flag!(no_report),
                Long("fail-under-lines") => parse_opt!(fail_under_lines),
                Long("fail-uncovered-lines") => parse_opt!(fail_uncovered_lines),
                Long("fail-uncovered-regions") => parse_opt!(fail_uncovered_regions),
                Long("fail-uncovered-functions") => parse_opt!(fail_uncovered_functions),
                Long("show-missing-lines") => parse_flag!(show_missing_lines),
                Long("include-build-script") => parse_flag!(include_build_script),

                // show-env options
                Long("export-prefix") => parse_flag!(export_prefix),

                Short('h') | Long("help") => {
                    print!("{}", Subcommand::help_text(subcommand));
                    std::process::exit(0);
                }
                Short('V') | Long("version") => {
                    if subcommand.is_none() {
                        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
                        std::process::exit(0);
                    } else {
                        unexpected("--version")?;
                    }
                }

                // passthrough
                Long(_) => passthrough!(),
                Short(flag) => {
                    if matches!(flag, 'q' | 'r') {
                        // To handle combined short flags properly, handle known
                        // short flags without value as special cases.
                        cargo_args.push(format!("-{flag}"));
                    } else {
                        passthrough!();
                    }
                }
                Value(val) => {
                    let val = val.parse::<String>()?;
                    if subcommand.is_none() {
                        if let Ok(v) = val.parse() {
                            subcommand = Some(v);
                            if subcommand == Some(Subcommand::Demangle) {
                                if let Some(arg) = parser.next()? {
                                    return Err(arg.unexpected().into());
                                }
                            }
                        } else {
                            cargo_args.push(val);
                        }
                    } else {
                        cargo_args.push(val);
                    }
                }
            }
        }

        term::set_coloring(&mut color);

        let subcommand = subcommand.unwrap_or(Subcommand::Test);

        // unexpected options
        if export_prefix && subcommand != Subcommand::ShowEnv {
            unexpected("--export-prefix")?;
        }
        // TODO: check
        match subcommand {
            Subcommand::Test => {}
            Subcommand::Run => {}
            Subcommand::ShowEnv => {}
            Subcommand::Clean => {}
            Subcommand::Nextest => {
                if doc || doctests {
                    bail!("doctest is not supported for nextest");
                }
            }
            Subcommand::Demangle => {}
        }

        // requires
        if !exclude.is_empty() && !workspace {
            // TODO: This is the same behavior as cargo, but should we allow it to be used
            // in the root of a virtual workspace as well?
            requires("--exclude", &["--workspace"])?;
        }
        if coverage_target_only && target.is_none() {
            requires("--coverage-target-only", &["--target"])?;
        }

        // conflicts
        if no_report && no_run {
            conflicts("--no-report", "--no-run")?;
        }
        if no_report || no_run {
            let flag = if no_report { "--no-report" } else { "--no-run" };
            if no_clean {
                // --no-report/--no-run implicitly enable --no-clean.
                conflicts(flag, "--no-clean")?;
            }
            no_clean = true;
        }
        if ignore_run_fail && no_fail_fast {
            // --ignore-run-fail implicitly enable --no-fail-fast.
            conflicts("--ignore-run-fail", "--no-fail-fast")?;
        }
        if doc || doctests {
            let flag = if doc { "--doc" } else { "--doctests" };
            if lib {
                conflicts(flag, "--lib")?;
            }
            if !bin.is_empty() {
                conflicts(flag, "--bin")?;
            }
            if bins {
                conflicts(flag, "--bins")?;
            }
            if !example.is_empty() {
                conflicts(flag, "--example")?;
            }
            if examples {
                conflicts(flag, "--examples")?;
            }
            if !test.is_empty() {
                conflicts(flag, "--test")?;
            }
            if tests {
                conflicts(flag, "--tests")?;
            }
            if !bench.is_empty() {
                conflicts(flag, "--bench")?;
            }
            if benches {
                conflicts(flag, "--benches")?;
            }
            if all_targets {
                conflicts(flag, "--all-targets")?;
            }
        }
        if !package.is_empty() && workspace {
            // cargo allows the combination of --package and --workspace, but
            // we reject it because the situation where both flags are specified is odd.
            conflicts("--package", "--workspace")?;
        }
        if lcov {
            let flag = "--lcov";
            if json {
                conflicts(flag, "--json")?;
            }
        }
        if text {
            let flag = "--text";
            if json {
                conflicts(flag, "--json")?;
            }
            if lcov {
                conflicts(flag, "--lcov")?;
            }
        }
        if html || open {
            let flag = if html { "--html" } else { "--open" };
            if json {
                conflicts(flag, "--json")?;
            }
            if lcov {
                conflicts(flag, "--lcov")?;
            }
            if text {
                conflicts(flag, "--text")?;
            }
        }
        if summary_only || output_path.is_some() {
            let flag = if summary_only { "--summary-only" } else { "--output-path" };
            if html {
                conflicts(flag, "--html")?;
            }
            if open {
                conflicts(flag, "--open")?;
            }
        }
        if output_dir.is_some() {
            let flag = "--output-dir";
            if json {
                conflicts(flag, "--json")?;
            }
            if lcov {
                conflicts(flag, "--lcov")?;
            }
            if output_path.is_some() {
                conflicts(flag, "--output-path")?;
            }
        }

        // forbid_empty_values
        if ignore_filename_regex.as_deref() == Some("") {
            bail!("empty string is not allowed in --ignore-filename-regex")
        }
        if output_path.as_deref() == Some(Utf8Path::new("")) {
            bail!("empty string is not allowed in --output-path")
        }
        if output_dir.as_deref() == Some(Utf8Path::new("")) {
            bail!("empty string is not allowed in --output-dir")
        }

        term::verbose::set(verbose != 0);
        // If `-vv` is passed, propagate `-v` to cargo.
        if verbose > 1 {
            cargo_args.push(format!("-{}", "v".repeat(verbose - 1)));
        }

        Ok(Self {
            subcommand,
            cov: LlvmCovOptions {
                json,
                lcov,
                text,
                html,
                open,
                summary_only,
                output_path,
                output_dir,
                failure_mode,
                ignore_filename_regex,
                disable_default_ignore_filename_regex,
                hide_instantiations,
                no_cfg_coverage,
                no_cfg_coverage_nightly,
                no_report,
                fail_under_lines,
                fail_uncovered_lines,
                fail_uncovered_regions,
                fail_uncovered_functions,
                show_missing_lines,
                include_build_script,
            },
            show_env: ShowEnvOptions { export_prefix },
            doctests,
            no_run,
            ignore_run_fail,
            lib,
            bin,
            bins,
            example,
            examples,
            test,
            tests,
            bench,
            benches,
            all_targets,
            doc,
            workspace,
            exclude,
            exclude_from_test,
            exclude_from_report,
            build: BuildOptions {
                jobs,
                release,
                profile,
                target,
                coverage_target_only,
                verbose: verbose.try_into().unwrap_or(u8::MAX),
                color,
                remap_path_prefix,
                include_ffi,
                no_clean,
            },
            manifest: ManifestOptions { manifest_path },
            cargo_args,
            rest,
        })
    }

    pub(crate) fn cov(&mut self) -> LlvmCovOptions {
        mem::take(&mut self.cov)
    }

    pub(crate) fn build(&mut self) -> BuildOptions {
        mem::take(&mut self.build)
    }

    pub(crate) fn manifest(&mut self) -> ManifestOptions {
        mem::take(&mut self.manifest)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Subcommand {
    Test,

    /// Run a binary or example and generate coverage report.
    Run,

    /// Remove artifacts that cargo-llvm-cov has generated in the past
    Clean,

    /// Output the environment set by cargo-llvm-cov to build Rust projects.
    ShowEnv,

    /// Run tests with cargo nextest
    Nextest,

    // internal (unstable)
    Demangle,
}

static CARGO_LLVM_COV_USAGE: &str = include_str!("../docs/cargo-llvm-cov.txt");
static CARGO_LLVM_COV_RUN_USAGE: &str = include_str!("../docs/cargo-llvm-cov-run.txt");
static CARGO_LLVM_COV_CLEAN_USAGE: &str = include_str!("../docs/cargo-llvm-cov-clean.txt");
static CARGO_LLVM_COV_SHOW_ENV_USAGE: &str = include_str!("../docs/cargo-llvm-cov-show-env.txt");
static CARGO_LLVM_COV_NEXTEST_USAGE: &str = include_str!("../docs/cargo-llvm-cov-nextest.txt");

impl Subcommand {
    fn help_text(subcommand: Option<Self>) -> &'static str {
        match subcommand {
            None | Some(Self::Test) => CARGO_LLVM_COV_USAGE,
            Some(Self::Run) => CARGO_LLVM_COV_RUN_USAGE,
            Some(Self::Clean) => CARGO_LLVM_COV_CLEAN_USAGE,
            Some(Self::ShowEnv) => CARGO_LLVM_COV_SHOW_ENV_USAGE,
            Some(Self::Nextest) => CARGO_LLVM_COV_NEXTEST_USAGE,
            Some(Self::Demangle) => "", // internal API
        }
    }
}

impl FromStr for Subcommand {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            // "test" | "t" => Ok(Self::Test),
            "run" | "r" => Ok(Self::Run),
            "show-env" => Ok(Self::ShowEnv),
            "clean" => Ok(Self::Clean),
            "nextest" => Ok(Self::Nextest),
            "demangle" => Ok(Self::Demangle),
            _ => bail!("unrecognized subcommand {s}"),
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct LlvmCovOptions {
    /// Export coverage data in "json" format
    ///
    /// If --output-path is not specified, the report will be printed to stdout.
    ///
    /// This internally calls `llvm-cov export -format=text`.
    /// See <https://llvm.org/docs/CommandGuide/llvm-cov.html#llvm-cov-export> for more.
    pub(crate) json: bool,
    /// Export coverage data in "lcov" format
    ///
    /// If --output-path is not specified, the report will be printed to stdout.
    ///
    /// This internally calls `llvm-cov export -format=lcov`.
    /// See <https://llvm.org/docs/CommandGuide/llvm-cov.html#llvm-cov-export> for more.
    pub(crate) lcov: bool,

    /// Generate coverage report in “text” format
    ///
    /// If --output-path or --output-dir is not specified, the report will be printed to stdout.
    ///
    /// This internally calls `llvm-cov show -format=text`.
    /// See <https://llvm.org/docs/CommandGuide/llvm-cov.html#llvm-cov-show> for more.
    pub(crate) text: bool,
    /// Generate coverage report in "html" format
    ///
    /// If --output-dir is not specified, the report will be generated in `target/llvm-cov/html` directory.
    ///
    /// This internally calls `llvm-cov show -format=html`.
    /// See <https://llvm.org/docs/CommandGuide/llvm-cov.html#llvm-cov-show> for more.
    pub(crate) html: bool,
    /// Generate coverage reports in "html" format and open them in a browser after the operation.
    ///
    /// See --html for more.
    pub(crate) open: bool,

    /// Export only summary information for each file in the coverage data
    ///
    /// This flag can only be used together with either --json or --lcov.
    // If the format flag is not specified, this flag is no-op because the only summary is displayed anyway.
    pub(crate) summary_only: bool,
    /// Specify a file to write coverage data into.
    ///
    /// This flag can only be used together with --json, --lcov, or --text.
    /// See --output-dir for --html and --open.
    pub(crate) output_path: Option<Utf8PathBuf>,
    /// Specify a directory to write coverage report into (default to `target/llvm-cov`).
    ///
    /// This flag can only be used together with --text, --html, or --open.
    /// See also --output-path.
    // If the format flag is not specified, this flag is no-op.
    pub(crate) output_dir: Option<Utf8PathBuf>,

    /// Fail if `any` or `all` profiles cannot be merged (default to `any`)
    pub(crate) failure_mode: Option<String>,
    /// Skip source code files with file paths that match the given regular expression.
    pub(crate) ignore_filename_regex: Option<String>,
    // For debugging (unstable)
    pub(crate) disable_default_ignore_filename_regex: bool,
    /// Hide instantiations from report
    pub(crate) hide_instantiations: bool,
    /// Unset cfg(coverage), which is enabled when code is built using cargo-llvm-cov.
    pub(crate) no_cfg_coverage: bool,
    /// Unset cfg(coverage_nightly), which is enabled when code is built using cargo-llvm-cov and nightly compiler.
    pub(crate) no_cfg_coverage_nightly: bool,
    /// Run tests, but don't generate coverage report
    pub(crate) no_report: bool,
    /// Exit with a status of 1 if the total line coverage is less than MIN percent.
    pub(crate) fail_under_lines: Option<f64>,
    /// Exit with a status of 1 if the uncovered lines are greater than MAX.
    pub(crate) fail_uncovered_lines: Option<u64>,
    /// Exit with a status of 1 if the uncovered regions are greater than MAX.
    pub(crate) fail_uncovered_regions: Option<u64>,
    /// Exit with a status of 1 if the uncovered functions are greater than MAX.
    pub(crate) fail_uncovered_functions: Option<u64>,
    /// Show lines with no coverage.
    pub(crate) show_missing_lines: bool,
    /// Include build script in coverage report.
    pub(crate) include_build_script: bool,
}

impl LlvmCovOptions {
    pub(crate) const fn show(&self) -> bool {
        self.text || self.html
    }
}

#[derive(Debug, Default)]
pub(crate) struct BuildOptions {
    /// Number of parallel jobs, defaults to # of CPUs
    // Max value is u32::MAX: https://github.com/rust-lang/cargo/blob/0.62.0/src/cargo/util/command_prelude.rs#L356
    pub(crate) jobs: Option<u32>,
    /// Build artifacts in release mode, with optimizations
    pub(crate) release: bool,
    /// Build artifacts with the specified profile
    pub(crate) profile: Option<String>,
    // /// Space or comma separated list of features to activate
    // pub(crate) features: Vec<String>,
    // /// Activate all available features
    // pub(crate) all_features: bool,
    // /// Do not activate the `default` feature
    // pub(crate) no_default_features: bool,
    /// Build for the target triple
    ///
    /// When this option is used, coverage for proc-macro and build script will
    /// not be displayed because cargo does not pass RUSTFLAGS to them.
    pub(crate) target: Option<String>,
    /// Activate coverage reporting only for the target triple
    ///
    /// Activate coverage reporting only for the target triple specified via `--target`.
    /// This is important, if the project uses multiple targets via the cargo
    /// bindeps feature, and not all targets can use `instrument-coverage`,
    /// e.g. a microkernel, or an embedded binary.
    pub(crate) coverage_target_only: bool,
    // TODO: Currently, we are using a subdirectory of the target directory as
    //       the actual target directory. What effect should this option have
    //       on its behavior?
    // /// Directory for all generated artifacts
    // target_dir: Option<Utf8PathBuf>,
    /// Use verbose output
    ///
    /// Use -vv (-vvv) to propagate verbosity to cargo.
    pub(crate) verbose: u8,
    /// Coloring
    // This flag will be propagated to both cargo and llvm-cov.
    pub(crate) color: Option<Coloring>,

    /// Use --remap-path-prefix for workspace root
    ///
    /// Note that this does not fully compatible with doctest.
    pub(crate) remap_path_prefix: bool,
    /// Include coverage of C/C++ code linked to Rust library/binary
    ///
    /// Note that `CC`/`CXX`/`LLVM_COV`/`LLVM_PROFDATA` environment variables
    /// must be set to Clang/LLVM compatible with the LLVM version used in rustc.
    // TODO: support specifying languages like: --include-ffi=c,  --include-ffi=c,c++
    pub(crate) include_ffi: bool,
    /// Build without cleaning any old build artifacts.
    ///
    /// Note that this can cause false positives/false negatives due to old build artifacts.
    pub(crate) no_clean: bool,
}

#[derive(Debug)]
pub(crate) struct ShowEnvOptions {
    /// Prepend "export " to each line, so that the output is suitable to be sourced by bash.
    pub(crate) export_prefix: bool,
}

// https://doc.rust-lang.org/nightly/cargo/commands/cargo-test.html#manifest-options
#[derive(Debug, Default)]
pub(crate) struct ManifestOptions {
    /// Path to Cargo.toml
    pub(crate) manifest_path: Option<Utf8PathBuf>,
}

fn format_flag(flag: &lexopt::Arg<'_>) -> String {
    match flag {
        Long(flag) => format!("--{flag}"),
        Short(flag) => format!("-{flag}"),
        Value(_) => unreachable!(),
    }
}

#[cold]
#[inline(never)]
fn multi_arg(flag: &lexopt::Arg<'_>) -> Result<()> {
    let flag = &format_flag(flag);
    bail!("The argument '{flag}' was provided more than once, but cannot be used multiple times");
}

// `flag` requires one of `requires`.
#[cold]
#[inline(never)]
fn requires(flag: &str, requires: &[&str]) -> Result<()> {
    let with = match requires.len() {
        0 => unreachable!(),
        1 => requires[0].to_string(),
        2 => format!("either {} or {}", requires[0], requires[1]),
        _ => {
            let mut with = String::new();
            for f in requires.iter().take(requires.len() - 1) {
                with += f;
                with += ", ";
            }
            with += "or ";
            with += requires.last().unwrap();
            with
        }
    };
    bail!("{flag} can only be used together with {with}");
}

#[cold]
#[inline(never)]
fn conflicts(a: &str, b: &str) -> Result<()> {
    bail!("{a} may not be used together with {b}");
}

#[cold]
#[inline(never)]
fn unexpected(arg: &str) -> Result<()> {
    bail!("found argument '{arg}' which wasn't expected, or isn't valid in this context");
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        io::Write,
        path::Path,
        process::{Command, Stdio},
    };

    use anyhow::Result;
    use fs_err as fs;

    use super::*;

    #[track_caller]
    fn assert_diff(expected_path: impl AsRef<Path>, actual: impl AsRef<str>) {
        let actual = actual.as_ref();
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let manifest_dir =
            manifest_dir.strip_prefix(env::current_dir().unwrap()).unwrap_or(manifest_dir);
        let expected_path = &manifest_dir.join(expected_path);
        if !expected_path.is_file() {
            fs::write(expected_path, "").unwrap();
        }
        let expected = fs::read_to_string(expected_path).unwrap();
        if expected != actual {
            if env::var_os("CI").is_some() {
                let mut child = Command::new("git")
                    .args(["--no-pager", "diff", "--no-index", "--"])
                    .arg(expected_path)
                    .arg("-")
                    .stdin(Stdio::piped())
                    .spawn()
                    .unwrap();
                child.stdin.as_mut().unwrap().write_all(actual.as_bytes()).unwrap();
                assert!(!child.wait().unwrap().success());
                // patch -p1 <<'EOF' ... EOF
                panic!("assertion failed; please run test locally and commit resulting changes, or apply above diff as patch");
            } else {
                fs::write(expected_path, actual).unwrap();
            }
        }
    }

    #[test]
    fn update_readme() -> Result<()> {
        let new = CARGO_LLVM_COV_USAGE;
        let path = &Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md");
        let base = fs::read_to_string(path)?;
        let mut out = String::with_capacity(base.capacity());
        let mut lines = base.lines();
        let mut start = false;
        let mut end = false;
        while let Some(line) = lines.next() {
            out.push_str(line);
            out.push('\n');
            if line == "<!-- readme-long-help:start -->" {
                start = true;
                out.push_str("```console\n");
                out.push_str("$ cargo llvm-cov --help\n");
                out.push_str(new);
                for line in &mut lines {
                    if line == "<!-- readme-long-help:end -->" {
                        out.push_str("```\n");
                        out.push_str(line);
                        out.push('\n');
                        end = true;
                        break;
                    }
                }
            }
        }
        if start && end {
            assert_diff(path, out);
        } else if start {
            panic!("missing `<!-- readme-long-help:end -->` comment in README.md");
        } else {
            panic!("missing `<!-- readme-long-help:start -->` comment in README.md");
        }
        Ok(())
    }
}
