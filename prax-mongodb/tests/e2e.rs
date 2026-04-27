//! End-to-end tests for prax-mongodb against a live MongoDB server.
//!
//! Gated by `PRAX_E2E=1` and requires `MONGODB_URL`.
//!
//! ```sh
//! docker compose up -d mongodb
//! docker compose run --rm test-mongodb
//! ```

#![cfg(test)]

use std::sync::atomic::{AtomicU32, Ordering};

use bson::{Document, doc, oid::ObjectId};
use futures::TryStreamExt;
use prax_mongodb::MongoClient;
use serde::{Deserialize, Serialize};

static COLL_COUNTER: AtomicU32 = AtomicU32::new(0);

fn unique_collection(prefix: &str) -> String {
    let n = COLL_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    format!("e2e_{prefix}_{pid}_{n}")
}

fn skip_unless_e2e() -> Option<String> {
    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        return None;
    }
    std::env::var("MONGODB_URL").ok()
}

async fn client() -> MongoClient {
    let url = skip_unless_e2e().expect("PRAX_E2E=1 and MONGODB_URL required");
    MongoClient::builder()
        .uri(url)
        // `prax_test` is the DB the compose healthcheck already provisions.
        .database("prax_test")
        .build()
        .await
        .expect("connect to mongodb")
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Widget {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    id: Option<ObjectId>,
    name: String,
    qty: i32,
}

#[tokio::test]
#[ignore = "requires running MongoDB via docker-compose"]
async fn e2e_crud_roundtrip() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let client = client().await;
    let name = unique_collection("crud");
    let col = client.collection::<Widget>(&name);

    // drop if leftover from a prior aborted run
    let _ = client.drop_collection(&name).await;

    // INSERT
    let insert = col
        .insert_one(
            Widget {
                id: None,
                name: "gadget".into(),
                qty: 7,
            },
            None,
        )
        .await
        .expect("insert");
    let id = insert.inserted_id.as_object_id().expect("ObjectId");

    // FIND one
    let found = col
        .find_one(doc! { "_id": id }, None)
        .await
        .expect("find_one")
        .expect("row exists");
    assert_eq!(found.name, "gadget");
    assert_eq!(found.qty, 7);

    // UPDATE — $set
    let res = col
        .update_one(doc! { "_id": id }, doc! { "$set": { "qty": 42 } }, None)
        .await
        .expect("update");
    assert_eq!(res.matched_count, 1);
    assert_eq!(res.modified_count, 1);

    let updated = col
        .find_one(doc! { "_id": id }, None)
        .await
        .expect("find_one after update")
        .expect("row exists");
    assert_eq!(updated.qty, 42);

    // DELETE
    let res = col
        .delete_one(doc! { "_id": id }, None)
        .await
        .expect("delete");
    assert_eq!(res.deleted_count, 1);

    // verify
    let none = col
        .find_one(doc! { "_id": id }, None)
        .await
        .expect("find_one after delete");
    assert!(none.is_none());

    client.drop_collection(&name).await.expect("cleanup");
}

#[tokio::test]
#[ignore = "requires running MongoDB via docker-compose"]
async fn e2e_bulk_insert_and_filter() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let client = client().await;
    let name = unique_collection("bulk");
    let col = client.collection::<Widget>(&name);
    let _ = client.drop_collection(&name).await;

    let docs: Vec<Widget> = (0..20_i32)
        .map(|i| Widget {
            id: None,
            name: if i % 2 == 0 { "even" } else { "odd" }.into(),
            qty: i,
        })
        .collect();
    let result = col.insert_many(docs, None).await.expect("insert_many");
    assert_eq!(result.inserted_ids.len(), 20);

    // Filter: only even with qty >= 10
    let mut cursor = col
        .find(doc! { "name": "even", "qty": { "$gte": 10 } }, None)
        .await
        .expect("find");
    let mut widgets = Vec::new();
    while let Some(w) = cursor.try_next().await.expect("cursor next") {
        widgets.push(w);
    }
    let qtys: Vec<i32> = widgets.iter().map(|w| w.qty).collect();
    let mut sorted = qtys.clone();
    sorted.sort();
    assert_eq!(sorted, vec![10, 12, 14, 16, 18]);

    client.drop_collection(&name).await.expect("cleanup");
}

#[tokio::test]
#[ignore = "requires running MongoDB via docker-compose"]
async fn e2e_aggregation_group_by() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let client = client().await;
    let name = unique_collection("agg");
    let col = client.collection_doc(&name);
    let _ = client.drop_collection(&name).await;

    let docs: Vec<Document> = (0..30_i32)
        .map(|i| doc! { "category": if i % 3 == 0 { "a" } else if i % 3 == 1 { "b" } else { "c" }, "amount": i })
        .collect();
    col.insert_many(docs, None).await.expect("insert_many");

    let pipeline = vec![
        doc! { "$group": { "_id": "$category", "total": { "$sum": "$amount" } } },
        doc! { "$sort": { "_id": 1 } },
    ];
    let mut cursor = col.aggregate(pipeline, None).await.expect("aggregate");
    let mut results = Vec::new();
    while let Some(d) = cursor.try_next().await.expect("cursor next") {
        results.push(d);
    }

    assert_eq!(results.len(), 3);
    let total_all: i64 = results
        .iter()
        .map(|d| d.get_i32("total").map(i64::from).unwrap_or_default())
        .sum();
    assert_eq!(total_all, (0..30_i32).sum::<i32>() as i64);

    client.drop_collection(&name).await.expect("cleanup");
}

#[tokio::test]
#[ignore = "requires running MongoDB via docker-compose"]
async fn e2e_client_is_healthy_and_lists_collections() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let client = client().await;
    assert!(client.is_healthy().await);

    // Use a scratch collection so we don't depend on the init.js seed state.
    let name = unique_collection("listcolls");
    let col = client.collection_doc(&name);
    col.insert_one(doc! { "ping": 1 }, None)
        .await
        .expect("insert");

    let names = client.list_collections().await.expect("list_collections");
    assert!(
        names.iter().any(|n| n == &name),
        "expected {name} in {names:?}"
    );

    client.drop_collection(&name).await.expect("cleanup");
}
