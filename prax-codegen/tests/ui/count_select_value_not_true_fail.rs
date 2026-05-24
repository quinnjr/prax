// Fixture: count! with a non-true value inside a select: block
// Expected diagnostic: "must be `true`"

fn main() {
    let client = ();
    let _ = prax_codegen::count!(client.user, {
        select: { email: 5 },
    });
}
