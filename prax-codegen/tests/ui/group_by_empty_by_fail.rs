// Fixture: group_by! with an empty by: list
// Expected diagnostic: "group_by! requires at least one column in `by:`"

fn main() {
    let client = ();
    let _ = prax_codegen::group_by!(client.user, {
        by: [],
        _count: { _all: true },
    });
}
