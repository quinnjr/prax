// Fixture: aggregate! with an unknown column inside a _count block
// Expected diagnostic: "unknown column `notacol`"

fn main() {
    let client = ();
    let _ = prax_codegen::aggregate!(client.user, {
        _count: { notacol: true },
    });
}
