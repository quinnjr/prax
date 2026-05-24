// Fixture: group_by! with an unknown column in the by: list
// Expected diagnostic: "unknown column `notacol`"

fn main() {
    let client = ();
    let _ = prax_codegen::group_by!(client.user, {
        by: [notacol],
        _count: { _all: true },
    });
}
