// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

/// Kind of working tree state entity
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum Kind {
    Header,
    Ordinary,
    RenameOrCopy,
    Unmerged,
    Untracked,
    Ignored,
}

/// Whether the working tree state is regular or unmerged. This is used in interpreting combined dispositions.
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum State {
    Regular,
    Unmerged,
}

/// Represents the possible values of the 'X' and 'Y' columns from `git-status` output.
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum Disposition {
    Unmodified,         /* '.' = unmodified (besides v1 = ' ') */
    Modified,           /* M = modified */
    FileTypeChanged,    /* T = file type changed (regular file, symbolic link or submodule) */
    Added,              /* A = added */
    Deleted,            /* D = deleted */
    Renamed,            /* R = renamed */
    Copied,             /* C = copied (if config option status.renames is set to "copies") */
    UpdatedButUnmerged, /* U = updated but unmerged */
    Untracked,          /* ? = untracked */
    Ignored,            /* ! = ignored */
}

impl TryFrom<char> for Disposition {
    type Error = anyhow::Error;

    fn try_from(value: char) -> Result<Self, Self::Error> {
        match value {
            '.' => Ok(Disposition::Unmodified),
            'M' => Ok(Disposition::Modified),
            'T' => Ok(Disposition::FileTypeChanged),
            'A' => Ok(Disposition::Added),
            'D' => Ok(Disposition::Deleted),
            'R' => Ok(Disposition::Renamed),
            'C' => Ok(Disposition::Copied),
            'U' => Ok(Disposition::UpdatedButUnmerged),
            '?' => Ok(Disposition::Untracked),
            '!' => Ok(Disposition::Ignored),
            _ => Err(anyhow::anyhow!(
                "Unexpected disposition character '{}'",
                value
            )),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum DispositionInterpretation {
    // Normal state
    NotUpdated,
    UpdatedInIndex,
    TypeChangedInIndex,
    AddedToIndex,
    DeletedFromIndex,
    RenamedInIndex,
    CopiedInIndex,
    // IndexAndWorkingTreeMatches, // Disabled for now because it is unreachable below.
    WorkTreeChangedSinceIndex,
    TypeChangedInWorkTreeSinceIndex,
    DeletedInWorkTree,
    RenamedInWorkTree,
    CopiedInWorkTree,

    // Unmerged state
    UnmergedBothDeleted,
    UnmergedAddedByUs,
    UnmergedDeletedByThem,
    UnmergedAddedByThem,
    UnmergedDeletedByUs,
    UnmergedBothAdded,
    UnmergedBothModified,

    // Specials
    Untracked,
    Ignored,
}

#[cfg(test)]
fn assert_disposition_interpretation(
    x_values: Vec<Disposition>,
    y_values: Vec<Disposition>,
    expected: DispositionInterpretation,
) {
    for x in x_values.iter() {
        for y in y_values.iter() {
            assert_eq!(
                DispositionInterpretation::try_from((State::Regular, *x, *y)).unwrap(),
                expected,
                "\n state: {:?},\n     x: {:?},\n     y: {:?}",
                State::Regular,
                *x,
                *y
            );
        }
    }
}

#[test]
fn test_regular_disposition_interpretations_not_updated() {
    // X          Y     Meaning
    // -------------------------------------------------
    //          [AMD]   not updated
    assert_disposition_interpretation(
        vec![Disposition::Unmodified],
        vec![
            Disposition::Added,
            Disposition::Modified,
            Disposition::Deleted,
        ],
        DispositionInterpretation::NotUpdated,
    );
}

#[test]
fn test_regular_disposition_interpretations_updated_in_index() {
    // X          Y     Meaning
    // -------------------------------------------------
    // M        [ MTD]  updated in index
    assert_disposition_interpretation(
        vec![Disposition::Modified],
        vec![
            Disposition::Unmodified,
            Disposition::Modified,
            Disposition::FileTypeChanged,
            Disposition::Deleted,
        ],
        DispositionInterpretation::UpdatedInIndex,
    );
}

#[test]
fn test_regular_disposition_interpretations_type_changed_in_index() {
    // X          Y     Meaning
    // -------------------------------------------------
    // A        [ MTD]  added to index
    assert_disposition_interpretation(
        vec![Disposition::FileTypeChanged],
        vec![
            Disposition::Unmodified,
            Disposition::Modified,
            Disposition::FileTypeChanged,
            Disposition::Deleted,
        ],
        DispositionInterpretation::TypeChangedInIndex,
    );
}

#[test]
fn test_regular_disposition_interpretations_added_to_index() {
    // X          Y     Meaning
    // -------------------------------------------------
    // A        [ MTD]  added to index
    assert_disposition_interpretation(
        vec![Disposition::Added],
        vec![
            Disposition::Unmodified,
            Disposition::Modified,
            Disposition::FileTypeChanged,
            Disposition::Deleted,
        ],
        DispositionInterpretation::AddedToIndex,
    );
}

#[test]
fn test_regular_disposition_interpretations_deleted_from_index() {
    // X          Y     Meaning
    // -------------------------------------------------
    // D                deleted from index
    assert_disposition_interpretation(
        vec![Disposition::Deleted],
        vec![Disposition::Unmodified],
        DispositionInterpretation::DeletedFromIndex,
    );
}

#[test]
fn test_regular_disposition_interpretations_renamed_in_index() {
    // X          Y     Meaning
    // -------------------------------------------------
    // R        [ MTD]  renamed in index
    assert_disposition_interpretation(
        vec![Disposition::Renamed],
        vec![
            Disposition::Unmodified,
            Disposition::Modified,
            Disposition::FileTypeChanged,
            Disposition::Deleted,
        ],
        DispositionInterpretation::RenamedInIndex,
    );
}

#[test]
fn test_regular_disposition_interpretations_copied_in_index() {
    // X          Y     Meaning
    // -------------------------------------------------
    // C        [ MTD]  copied in index
    assert_disposition_interpretation(
        vec![Disposition::Copied],
        vec![
            Disposition::Unmodified,
            Disposition::Modified,
            Disposition::FileTypeChanged,
            Disposition::Deleted,
        ],
        DispositionInterpretation::CopiedInIndex,
    );
}

#[ignore]
#[test]
fn test_regular_disposition_interpretations_work_tree_changed_since_index() {
    // X          Y     Meaning
    // -------------------------------------------------
    // [ MTARC]    M    work tree changed since index
    assert_disposition_interpretation(
        vec![
            Disposition::Unmodified,
            Disposition::Modified,
            Disposition::FileTypeChanged,
            Disposition::Added,
            Disposition::Renamed,
            Disposition::Copied,
        ],
        vec![Disposition::Modified],
        DispositionInterpretation::WorkTreeChangedSinceIndex,
    );
}

#[ignore]
#[test]
fn test_regular_disposition_interpretations_type_changed_in_work_tree_changed_since_index() {
    // X          Y     Meaning
    // -------------------------------------------------
    // [ MTARC]    T    type changed in work tree since index
    assert_disposition_interpretation(
        vec![
            Disposition::Unmodified,
            Disposition::Modified,
            Disposition::FileTypeChanged,
            Disposition::Added,
            Disposition::Renamed,
            Disposition::Copied,
        ],
        vec![Disposition::FileTypeChanged],
        DispositionInterpretation::TypeChangedInWorkTreeSinceIndex,
    );
}

impl TryFrom<(State, Disposition, Disposition)> for DispositionInterpretation {
    type Error = anyhow::Error;

    /*
    From manpage git-status(1) § "Short Format", see https://git-scm.com/docs/git-status

    X          Y     Meaning
    -------------------------------------------------
            [AMD]   not updated
    M        [ MTD]  updated in index
    T        [ MTD]  type changed in index
    A        [ MTD]  added to index
    D                deleted from index
    R        [ MTD]  renamed in index
    C        [ MTD]  copied in index
    [MTARC]          index and work tree matches
    [ MTARC]    M    work tree changed since index
    [ MTARC]    T    type changed in work tree since index
    [ MTARC]    D    deleted in work tree
                R    renamed in work tree
                C    copied in work tree
    -------------------------------------------------
    D           D    unmerged, both deleted
    A           U    unmerged, added by us
    U           D    unmerged, deleted by them
    U           A    unmerged, added by them
    D           U    unmerged, deleted by us
    A           A    unmerged, both added
    U           U    unmerged, both modified
    -------------------------------------------------
    ?           ?    untracked
    !           !    ignored
    -------------------------------------------------
    */
    /// Interpret a tuple of repo state and a pair of dispositions according to the table defined in git-status(1) § "Short Format"
    fn try_from(value: (State, Disposition, Disposition)) -> Result<Self, Self::Error> {
        let (state, x, y) = value;
        match (state, x, y) {
            (
                State::Regular,
                Disposition::Unmodified,
                Disposition::Added | Disposition::Modified | Disposition::Deleted,
            ) => Ok(DispositionInterpretation::NotUpdated),
            (
                State::Regular,
                Disposition::Modified,
                Disposition::Unmodified
                | Disposition::Modified
                | Disposition::FileTypeChanged
                | Disposition::Deleted,
            ) => Ok(DispositionInterpretation::UpdatedInIndex),
            (
                State::Regular,
                Disposition::FileTypeChanged,
                Disposition::Unmodified
                | Disposition::Modified
                | Disposition::FileTypeChanged
                | Disposition::Deleted,
            ) => Ok(DispositionInterpretation::TypeChangedInIndex),
            (
                State::Regular,
                Disposition::Added,
                Disposition::Unmodified
                | Disposition::Modified
                | Disposition::FileTypeChanged
                | Disposition::Deleted,
            ) => Ok(DispositionInterpretation::AddedToIndex),
            (State::Regular, Disposition::Deleted, _) => {
                Ok(DispositionInterpretation::DeletedFromIndex)
            }
            (
                State::Regular,
                Disposition::Renamed,
                Disposition::Unmodified
                | Disposition::Modified
                | Disposition::FileTypeChanged
                | Disposition::Deleted,
            ) => Ok(DispositionInterpretation::RenamedInIndex),
            (
                State::Regular,
                Disposition::Copied,
                Disposition::Unmodified
                | Disposition::Modified
                | Disposition::FileTypeChanged
                | Disposition::Deleted,
            ) => Ok(DispositionInterpretation::CopiedInIndex),
            // (
            //     State::Regular,
            //     Disposition::Modified
            //     | Disposition::FileTypeChanged
            //     | Disposition::Added
            //     | Disposition::Renamed
            //     | Disposition::Copied,
            //     Disposition::Unmodified,
            // ) =>
            // /* TODO: Figure out why this pattern is not matched */
            // {
            //     Ok(DispositionInterpretation::IndexAndWorkingTreeMatches)
            // }
            (State::Regular, _, Disposition::Modified) => {
                Ok(DispositionInterpretation::WorkTreeChangedSinceIndex)
            }
            (State::Regular, _, Disposition::FileTypeChanged) => {
                Ok(DispositionInterpretation::TypeChangedInWorkTreeSinceIndex)
            }
            (State::Regular, _, Disposition::Deleted) => {
                Ok(DispositionInterpretation::DeletedInWorkTree)
            }
            (State::Regular, Disposition::Unmodified, Disposition::Renamed) => {
                Ok(DispositionInterpretation::RenamedInWorkTree)
            }
            (State::Regular, Disposition::Unmodified, Disposition::Copied) => {
                Ok(DispositionInterpretation::CopiedInWorkTree)
            }
            (State::Unmerged, Disposition::Deleted, Disposition::Deleted) => {
                Ok(DispositionInterpretation::UnmergedBothDeleted)
            }
            (State::Unmerged, Disposition::Added, Disposition::UpdatedButUnmerged) => {
                Ok(DispositionInterpretation::UnmergedAddedByUs)
            }
            (State::Unmerged, Disposition::UpdatedButUnmerged, Disposition::Deleted) => {
                Ok(DispositionInterpretation::UnmergedDeletedByThem)
            }
            (State::Unmerged, Disposition::UpdatedButUnmerged, Disposition::Added) => {
                Ok(DispositionInterpretation::UnmergedAddedByThem)
            }
            (State::Unmerged, Disposition::Deleted, Disposition::UpdatedButUnmerged) => {
                Ok(DispositionInterpretation::UnmergedDeletedByUs)
            }
            (State::Unmerged, Disposition::Added, Disposition::Added) => {
                Ok(DispositionInterpretation::UnmergedBothAdded)
            }
            (State::Unmerged, Disposition::UpdatedButUnmerged, Disposition::UpdatedButUnmerged) => {
                Ok(DispositionInterpretation::UnmergedBothModified)
            }

            (_, Disposition::Untracked, Disposition::Untracked) => {
                Ok(DispositionInterpretation::Untracked)
            }
            (_, Disposition::Ignored, Disposition::Ignored) => {
                Ok(DispositionInterpretation::Ignored)
            }

            _ => Err(anyhow::anyhow!(
                "Unsupported disposition combination (state={:?}, X='{:?}', Y='{:?}'",
                &state,
                x,
                y
            )),
        }
    }
}
