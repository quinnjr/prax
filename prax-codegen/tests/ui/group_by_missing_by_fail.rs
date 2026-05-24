// Fixture: group_by! with no by: key at all
// Expected diagnostic: "group_by! requires a `by: [...]` list"

fn main() {
    let client = ();
    let _ = prax_codegen::group_by!(client.user, {
        _count: { _all: true },
    });
}
