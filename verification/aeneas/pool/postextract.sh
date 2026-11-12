#!/usr/bin/env bash
# Post-extraction fix for one Aeneas code-generation bug (run by ../run.sh
# after Aeneas, before the drift check and the Lean build). Argument: the
# generated library directory (lean/PoolAeneas).
#
# The bug (Aeneas nightly-2026.11.12-0d21a9e, Charon 454586e): a trait with two
# parent clauses for the SAME trait gets two structure fields with the SAME
# name. `core::num::nonzero::ZeroablePrimitive` is declared
#
#     pub unsafe trait ZeroablePrimitive: Sized + Copy + private::Sealed {
#         type NonZeroInner: Sized + Copy;
#     }
#
# and Charon lifts the associated type to a type parameter, so the trait has
# two `Copy` parent clauses (`Self: Copy`, `Self::NonZeroInner: Copy`).
# Aeneas names both `markerCopyInst`, which Lean rejects
# ("field 'markerCopyInst' has already been declared"). It is reached through
# `NonZeroU64::new`/`get` in `interest::global_sync`. See docs/pool.md
# ("Aeneas bug: duplicate parent-clause field").
#
# The fix renames the SECOND field to `markerCopyInst1` in exactly three
# places: the structure declaration and its `parentClauses` attribute
# (Types.lean), and the one instance (`u64`'s impl, Funs.lean). A field name
# is a label; the two fields keep their types, order and values, and no
# generated function reads either field (they are proofs that the marker
# trait `Copy` holds, with no computational content). So the renaming changes
# no definition's meaning. Each substitution must match exactly once, or the
# script fails, so an Aeneas upgrade that changes the output is noticed.
set -euo pipefail
dir=${1:?usage: postextract.sh <generated Lean library dir>}

perl -0 -i -e '
  my $types = "$ARGV[0]/Types.lean"; my $funs = "$ARGV[0]/Funs.lean";
  sub fix { my ($file, $from, $to) = @_;
    local $/; open my $fh, "<", $file or die "$file: $!"; my $s = <$fh>; close $fh;
    my $n = () = $s =~ /\Q$from\E/g;
    die "postextract.sh: expected exactly one match in $file, found $n:\n$from\n" unless $n == 1;
    $s =~ s/\Q$from\E/$to/;
    open $fh, ">", $file or die "$file: $!"; print $fh $s; close $fh; }
  fix($types,
    q{(parentClauses := ["markerCopyInst", "markerCopyInst"])},
    q{(parentClauses := ["markerCopyInst", "markerCopyInst1"])});
  fix($types,
    "  markerCopyInst : core.marker.Copy Self\n  markerCopyInst : core.marker.Copy Self_NonZeroInner\n",
    "  markerCopyInst : core.marker.Copy Self\n  markerCopyInst1 : core.marker.Copy Self_NonZeroInner\n");
  fix($funs,
    "  markerCopyInst := core.marker.CopyU64\n  markerCopyInst := core.num.niche_types.NonZeroU64Inner.Insts.CoreMarkerCopy\n",
    "  markerCopyInst := core.marker.CopyU64\n  markerCopyInst1 := core.num.niche_types.NonZeroU64Inner.Insts.CoreMarkerCopy\n");
' "$dir" </dev/null
echo "postextract.sh: renamed the duplicate ZeroablePrimitive field (Aeneas bug workaround)"
