// Copyright (c) Zefchain Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for the Hex application.

#![cfg(not(target_arch = "wasm32"))]

use hex_game::{HexAbi, Operation, Timeouts};
use linera_sdk::{
    linera_base_types::{
        AccountSecretKey, Amount, BlobType, ChainDescription, Secp256k1SecretKey, TimeDelta,
    },
    test::{ActiveChain, QueryOutcome, TestValidator},
};

#[test_log::test(tokio::test)]
async fn hex_game() {
    let key_pair1 = AccountSecretKey::generate();
    let key_pair2 = AccountSecretKey::Secp256k1(Secp256k1SecretKey::generate());

    let (validator, app_id, creation_chain) =
        TestValidator::with_current_application::<HexAbi, _, _>((), Timeouts::default()).await;

    let certificate = creation_chain
        .add_block(|block| {
            let operation = Operation::Start {
                board_size: 2,
                players: [key_pair1.public().into(), key_pair2.public().into()],
                fee_budget: Amount::ZERO,
                timeouts: None,
            };
            block.with_operation(app_id, operation);
        })
        .await;

    let block = certificate.inner().block();
    let description = block
        .created_blobs()
        .into_iter()
        .filter_map(|(blob_id, blob)| {
            (blob_id.blob_type == BlobType::ChainDescription)
                .then(|| bcs::from_bytes::<ChainDescription>(blob.content().bytes()).unwrap())
        })
        .next()
        .unwrap();
    let mut chain = ActiveChain::new(key_pair1.copy(), description, validator);

    chain
        .add_block(|block| {
            block.with_messages_from(&certificate);
            block.with_operation(app_id, Operation::MakeMove { x: 0, y: 0 });
        })
        .await;

    chain.set_key_pair(key_pair2.copy());
    chain
        .add_block(|block| {
            block.with_operation(app_id, Operation::MakeMove { x: 0, y: 1 });
        })
        .await;

    chain.set_key_pair(key_pair1.copy());
    chain
        .add_block(|block| {
            block.with_operation(app_id, Operation::MakeMove { x: 1, y: 1 });
        })
        .await;

    let QueryOutcome { response, .. } = chain.graphql_query(app_id, "query { winner }").await;
    assert!(response["winner"].is_null());

    chain.set_key_pair(key_pair2.copy());
    chain
        .add_block(|block| {
            block.with_operation(app_id, Operation::MakeMove { x: 1, y: 0 });
        })
        .await;

    let QueryOutcome { response, .. } = chain.graphql_query(app_id, "query { winner }").await;
    assert_eq!(Some("TWO"), response["winner"].as_str());
    assert!(chain.is_closed().await);
}

#[tokio::test]
async fn hex_game_clock() {
    let key_pair1 = AccountSecretKey::generate();
    let key_pair2 = AccountSecretKey::Secp256k1(Secp256k1SecretKey::generate());

    let timeouts = Timeouts {
        start_time: TimeDelta::from_secs(60),
        increment: TimeDelta::from_secs(30),
        block_delay: TimeDelta::from_secs(5),
    };

    let (validator, app_id, creation_chain) =
        TestValidator::with_current_application::<HexAbi, _, _>((), Timeouts::default()).await;

    let time = validator.clock().current_time();
    validator.clock().add(
        timeouts
            .block_delay
            .saturating_sub(TimeDelta::from_millis(1)),
    );

    let certificate = creation_chain
        .add_block(|block| {
            let operation = Operation::Start {
                board_size: 2,
                players: [key_pair1.public().into(), key_pair2.public().into()],
                fee_budget: Amount::ZERO,
                timeouts: None,
            };
            block.with_operation(app_id, operation).with_timestamp(time);
        })
        .await;

    let block = certificate.inner().block();
    let description = block
        .created_blobs()
        .into_iter()
        .filter_map(|(blob_id, blob)| {
            (blob_id.blob_type == BlobType::ChainDescription)
                .then(|| bcs::from_bytes::<ChainDescription>(blob.content().bytes()).unwrap())
        })
        .next()
        .unwrap();
    let mut chain = ActiveChain::new(key_pair1.copy(), description, validator.clone());

    chain
        .add_block(|block| {
            block
                .with_messages_from(&certificate)
                .with_operation(app_id, Operation::MakeMove { x: 0, y: 0 })
                .with_timestamp(time);
        })
        .await;

    validator.clock().add(TimeDelta::from_millis(1));

    // Block timestamp is too far behind.
    chain.set_key_pair(key_pair2.copy());
    assert!(chain
        .try_add_block(|block| {
            block
                .with_operation(app_id, Operation::MakeMove { x: 0, y: 1 })
                .with_timestamp(time);
        })
        .await
        .is_err());

    validator.clock().add(timeouts.start_time);
    let time = validator.clock().current_time();

    // Player 2 has timed out.
    assert!(chain
        .try_add_block(|block| {
            block
                .with_operation(app_id, Operation::MakeMove { x: 0, y: 1 })
                .with_timestamp(time);
        })
        .await
        .is_err());

    chain.set_key_pair(key_pair1.copy());
    chain
        .add_block(|block| {
            block
                .with_operation(app_id, Operation::ClaimVictory)
                .with_timestamp(time);
        })
        .await;

    let QueryOutcome { response, .. } = chain.graphql_query(app_id, "query { winner }").await;
    assert_eq!(Some("ONE"), response["winner"].as_str());
}
