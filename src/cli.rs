use argh::FromArgs;

#[derive(FromArgs, PartialEq, Debug)]
/// run scan phase only: scan all the packages and commit to database
#[argh(subcommand, name = "scan")]
pub(crate) struct PVectorScan {}

#[derive(FromArgs, PartialEq, Debug)]
/// run release phase only: generate Release files
#[argh(subcommand, name = "release")]
pub(crate) struct PVectorRelease {}

#[derive(FromArgs, PartialEq, Debug)]
/// run maintenance phase only: run database maintenance
#[argh(subcommand, name = "maintenance")]
pub(crate) struct PVectorMaintenance {}

#[derive(FromArgs, PartialEq, Debug)]
/// reset the database (all the existing data will be deleted)
#[argh(subcommand, name = "reset")]
pub(crate) struct PVectorReset {}

#[derive(FromArgs, PartialEq, Debug)]
/// run gc phase only: remove all the deleted branches
#[argh(subcommand, name = "gc")]
pub(crate) struct PVectorGC {}

#[derive(FromArgs, PartialEq, Debug)]
/// run a full cycle: equals to running scan, release, sync, analyze and gc
#[argh(subcommand, name = "full")]
pub(crate) struct PVectorFullCycle {}

#[derive(FromArgs, PartialEq, Debug)]
/// generate a new certificate used for signing
#[argh(subcommand, name = "gen-key")]
pub(crate) struct PVectorGenKey {}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand)]
pub(crate) enum PVectorCommand {
    Scan(PVectorScan),
    Release(PVectorRelease),
    Maintenance(PVectorMaintenance),
    Reset(PVectorReset),
    GC(PVectorGC),
    Full(PVectorFullCycle),
    GenKey(PVectorGenKey),
}

#[derive(FromArgs, PartialEq, Debug)]
/// P-Vector: Scanner for deb packages
pub(crate) struct PVector {
    /// specify the location of the config file
    #[argh(option, short = 'c')]
    pub config: String,
    #[argh(subcommand)]
    pub command: PVectorCommand,
}
