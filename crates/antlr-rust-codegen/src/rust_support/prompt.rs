// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
use std::io::{self, Write as _};

use anstream::{AutoStream, ColorChoice};
use anstyle::{AnsiColor, Effects, Style};

use super::identity::BundleIdentity;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrustDecision {
    Once,
    Revision,
    Repository,
    Abort,
}

pub(crate) fn ask(identity: &BundleIdentity) -> io::Result<TrustDecision> {
    let mut output = AutoStream::new(io::stderr(), ColorChoice::Auto);
    let mut input = io::stdin().lock();

    let warning = miette::miette!(
        severity = miette::Severity::Warning,
        help = "Approve only if you trust this source. The transform runs in a disposable copy, \
                but this trusted-source path is not a security sandbox.",
        "the grammar ships executable Rust target support"
    );
    writeln!(output, "\n{warning:?}")?;
    render_identity(&mut output, identity)?;
    render_choices(&mut output)?;
    read_decision(&mut input, &mut output)
}

fn render_identity(output: &mut impl io::Write, identity: &BundleIdentity) -> io::Result<()> {
    let label = Style::new().effects(Effects::BOLD);
    let value = Style::new().fg_color(Some(AnsiColor::Cyan.into()));
    writeln!(
        output,
        "  {label}Source{label:#}       {value}{}{value:#}",
        identity.source_label()
    )?;
    if let Some(revision) = &identity.revision {
        writeln!(
            output,
            "  {label}Revision{label:#}     {value}{revision}{value:#}"
        )?;
    }
    writeln!(
        output,
        "  {label}Fingerprint{label:#}  {value}{}{value:#}\n",
        identity.fingerprint
    )
}

fn render_choices(output: &mut impl io::Write) -> io::Result<()> {
    let number = Style::new()
        .fg_color(Some(AnsiColor::Cyan.into()))
        .effects(Effects::BOLD);
    writeln!(output, "  {number}1{number:#}  Trust once")?;
    writeln!(output, "  {number}2{number:#}  Trust this exact revision")?;
    writeln!(output, "  {number}3{number:#}  Trust this repository")?;
    writeln!(output, "  {number}4{number:#}  Abort")
}

fn read_decision(
    input: &mut impl io::BufRead,
    output: &mut impl io::Write,
) -> io::Result<TrustDecision> {
    let prompt = Style::new()
        .fg_color(Some(AnsiColor::Green.into()))
        .effects(Effects::BOLD);
    loop {
        write!(output, "\n{prompt}?{prompt:#} Select [1-4]: ")?;
        output.flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Ok(TrustDecision::Abort);
        }
        match line.trim() {
            "1" => return Ok(TrustDecision::Once),
            "2" => return Ok(TrustDecision::Revision),
            "3" => return Ok(TrustDecision::Repository),
            "4" => return Ok(TrustDecision::Abort),
            _ => writeln!(output, "  Enter 1, 2, 3, or 4.")?,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{TrustDecision, read_decision};

    #[test]
    fn selection_reprompts_and_accepts_a_scope() {
        let mut input = Cursor::new(b"no\n2\n");
        let mut output = Vec::new();

        let decision = read_decision(&mut input, &mut output).expect("prompt should be readable");

        assert_eq!(decision, TrustDecision::Revision);
        assert!(
            String::from_utf8(output)
                .expect("prompt output should be UTF-8")
                .contains("Enter 1, 2, 3, or 4.")
        );
    }
}
