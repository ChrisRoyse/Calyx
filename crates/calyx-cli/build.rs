//! Embeds the git build identity (issue #1108) — see `calyx-buildinfo`.

fn main() {
    calyx_buildinfo::emit();
}
