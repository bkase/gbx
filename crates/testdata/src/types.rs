//! Core type definitions for testdata metadata.

/// Target hardware profile for a ROM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RomModel {
    /// Original Game Boy / DMG hardware.
    Dmg,
    /// Game Boy Color / CGB hardware.
    Cgb,
    /// Super Game Boy hybrid hardware.
    Sgb,
    /// Fallback when the model is not known.
    Unknown,
}

/// High-level category describing what the ROM exercises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RomKind {
    /// CPU functional coverage.
    Cpu,
    /// Timer or instruction timing behaviour.
    Timing,
    /// PPU or visual correctness.
    Visual,
    /// APU or audio behaviour.
    Audio,
    /// Serial/IO tests that communicate through SB/SC.
    Serial,
    /// Fallback when the category is unknown.
    Unknown,
}

/// Expected completion signal for a ROM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expected {
    /// No expectation was recorded yet.
    Unknown,
    /// The ROM prints the provided ASCII payload over serial.
    SerialAscii(&'static str),
    /// The ROM uses the Mooneye Fibonacci register contract.
    MooneyeFib,
    /// The ROM should match an upstream reference screenshot.
    Screenshot,
}

/// Describes a single ROM entry exposed by the `testdata` crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomMeta {
    /// Test-suite grouping (top-level directory in the bundle).
    pub suite: &'static str,
    /// Friendly name of the ROM (filename without extension).
    pub name: &'static str,
    /// Normalized path relative to the bundle root.
    pub path: &'static str,
    /// Target hardware model.
    pub model: RomModel,
    /// Behavioural category for reporting.
    pub kind: RomKind,
    /// Expected success criteria.
    pub expected: Expected,
    /// SHA-256 digest of the ROM bytes.
    pub sha256: &'static str,
    /// Size of the ROM payload in bytes.
    pub size: u64,
    /// Whether the ROM is embedded when the `embed` feature is enabled.
    pub embed: bool,
}
