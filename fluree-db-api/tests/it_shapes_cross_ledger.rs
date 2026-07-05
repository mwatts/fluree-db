//! End-to-end cross-ledger SHACL shape enforcement.
//!
//! Data ledger D's `#config` declares `f:shapesSource` with
//! `f:ledger` pointing at model ledger M's shapes graph. The
//! cross-ledger dispatch happens at the API boundary
//! (`stage_with_config_shacl`): we resolve M's shapes to an
//! IRI-form wire artifact before staging, thread the wire into
//! `StagedShaclContext`, then at SHACL validation time compile
//! the wire against the *staged* `NamespaceRegistry` (which has
//! D's snapshot namespaces plus any IRIs the in-flight
//! transaction introduced).

#![cfg(all(feature = "native", feature = "shacl"))]

use crate::support::genesis_ledger;
use fluree_db_api::FlureeBuilder;
use serde_json::json;

fn config_graph_iri(ledger_id: &str) -> String {
    format!("urn:fluree:{ledger_id}#config")
}

#[tokio::test]
async fn data_ledger_tx_rejected_by_cross_ledger_shape() {
    let fluree = FlureeBuilder::memory().build_memory();

    let model_id = "test/cross-ledger-shapes/model:main";
    let model = genesis_ledger(&fluree, model_id);

    let shapes_graph_iri = "http://example.org/governance/shapes";
    let m_trig = format!(
        r"
        @prefix sh:   <http://www.w3.org/ns/shacl#> .
        @prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
        @prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .
        @prefix ex:   <http://example.org/ns/> .

        GRAPH <{shapes_graph_iri}> {{
            ex:PersonShape
                rdf:type        sh:NodeShape ;
                sh:targetClass  ex:Person ;
                sh:property     ex:pshape_name .
            ex:pshape_name
                sh:path     ex:name ;
                sh:minCount 1 ;
                sh:datatype xsd:string .
        }}
    "
    );
    fluree
        .stage_owned(model)
        .upsert_turtle(&m_trig)
        .execute()
        .await
        .expect("seed M shapes");

    let data_id = "test/cross-ledger-shapes/data:main";
    let data = genesis_ledger(&fluree, data_id);

    let config_iri = config_graph_iri(data_id);
    let r1 = fluree
        .stage_owned(data)
        .upsert_turtle(&format!(
            r"
            @prefix f:   <https://ns.flur.ee/db#> .
            @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .

            GRAPH <{config_iri}> {{
                <urn:cfg:main> rdf:type f:LedgerConfig .
                <urn:cfg:main> f:shaclDefaults <urn:cfg:shacl> .
                <urn:cfg:shacl> f:shaclEnabled true .
                <urn:cfg:shacl> f:shapesSource <urn:cfg:shapes-ref> .
                <urn:cfg:shapes-ref> rdf:type f:GraphRef ;
                                     f:graphSource <urn:cfg:shapes-src> .
                <urn:cfg:shapes-src> f:ledger <{model_id}> ;
                                     f:graphSelector <{shapes_graph_iri}> .
            }}
        "
        ))
        .execute()
        .await
        .expect("seed D cross-ledger SHACL config");
    let data = r1.ledger;

    // ex:Person without ex:name → must be rejected by M's shape.
    // This is the load-bearing assertion: the cross-ledger wire
    // must compile against the staged namespace registry (where
    // ex:Person is registered by the in-flight tx), not against
    // D's pre-stage snapshot (where ex: hasn't been allocated).
    let err = fluree
        .insert(
            data,
            &json!({
                "@context": {"ex": "http://example.org/ns/"},
                "@id": "ex:alice",
                "@type": "ex:Person"
            }),
        )
        .await
        .expect_err("violating Person under cross-ledger shape must be rejected");

    assert!(
        matches!(
            err,
            fluree_db_api::ApiError::Transact(fluree_db_transact::TransactError::ShaclViolation(_))
        ),
        "expected ShaclViolation from M's cross-ledger shape, got: {err:?}"
    );
}

#[tokio::test]
async fn data_ledger_tx_passes_when_cross_ledger_shape_satisfied() {
    let fluree = FlureeBuilder::memory().build_memory();

    let model_id = "test/cross-ledger-shapes/valid-model:main";
    let model = genesis_ledger(&fluree, model_id);

    let shapes_graph_iri = "http://example.org/governance/shapes";
    fluree
        .stage_owned(model)
        .upsert_turtle(&format!(
            r"
            @prefix sh:   <http://www.w3.org/ns/shacl#> .
            @prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
            @prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .
            @prefix ex:   <http://example.org/ns/> .

            GRAPH <{shapes_graph_iri}> {{
                ex:PersonShape
                    rdf:type        sh:NodeShape ;
                    sh:targetClass  ex:Person ;
                    sh:property     ex:pshape_name .
                ex:pshape_name
                    sh:path     ex:name ;
                    sh:minCount 1 ;
                    sh:datatype xsd:string .
            }}
        "
        ))
        .execute()
        .await
        .expect("seed M shapes");

    let data_id = "test/cross-ledger-shapes/valid-data:main";
    let data = genesis_ledger(&fluree, data_id);

    let config_iri = config_graph_iri(data_id);
    let r1 = fluree
        .stage_owned(data)
        .upsert_turtle(&format!(
            r"
            @prefix f:   <https://ns.flur.ee/db#> .
            @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .

            GRAPH <{config_iri}> {{
                <urn:cfg:main> rdf:type f:LedgerConfig .
                <urn:cfg:main> f:shaclDefaults <urn:cfg:shacl> .
                <urn:cfg:shacl> f:shaclEnabled true .
                <urn:cfg:shacl> f:shapesSource <urn:cfg:shapes-ref> .
                <urn:cfg:shapes-ref> rdf:type f:GraphRef ;
                                     f:graphSource <urn:cfg:shapes-src> .
                <urn:cfg:shapes-src> f:ledger <{model_id}> ;
                                     f:graphSelector <{shapes_graph_iri}> .
            }}
        "
        ))
        .execute()
        .await
        .expect("seed D config");
    let data = r1.ledger;

    // ex:bob has the required ex:name. Shape should accept.
    fluree
        .insert(
            data,
            &json!({
                "@context": {"ex": "http://example.org/ns/"},
                "@id": "ex:bob",
                "@type": "ex:Person",
                "ex:name": "Bob"
            }),
        )
        .await
        .expect("valid Person under cross-ledger shape must be accepted");
}

/// Cross-ledger `sh:class` value-set: model ledger M holds both the shape
/// (`sh:class ex:USState`) and the controlled vocabulary (`ex:illinois a
/// ex:USState`). Data ledger D references those value-set members. The shape is
/// enforced cross-ledger (via the wire); membership is resolved by querying M
/// live — the vocabulary is ABox and is NOT carried in the shapes wire.
#[tokio::test]
async fn cross_ledger_sh_class_value_set_resolved_against_model_ledger() {
    let fluree = FlureeBuilder::memory().build_memory();

    let model_id = "test/cross-ledger-shapes/vocab-model:main";
    let model = genesis_ledger(&fluree, model_id);

    let shapes_graph_iri = "http://example.org/governance/shapes";
    fluree
        .stage_owned(model)
        .upsert_turtle(&format!(
            r"
            @prefix sh:   <http://www.w3.org/ns/shacl#> .
            @prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
            @prefix ex:   <http://example.org/ns/> .

            GRAPH <{shapes_graph_iri}> {{
                ex:PersonShape
                    rdf:type        sh:NodeShape ;
                    sh:targetClass  ex:Person ;
                    sh:property     ex:pshape_state .
                ex:pshape_state
                    sh:path  ex:homeState ;
                    sh:class ex:USState .

                ex:illinois rdf:type ex:USState .
                ex:iowa     rdf:type ex:USState .
            }}
        "
        ))
        .execute()
        .await
        .expect("seed M shapes + controlled vocabulary");

    let data_id = "test/cross-ledger-shapes/vocab-data:main";
    let data = genesis_ledger(&fluree, data_id);

    let config_iri = config_graph_iri(data_id);
    let r1 = fluree
        .stage_owned(data)
        .upsert_turtle(&format!(
            r"
            @prefix f:   <https://ns.flur.ee/db#> .
            @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .

            GRAPH <{config_iri}> {{
                <urn:cfg:main> rdf:type f:LedgerConfig .
                <urn:cfg:main> f:shaclDefaults <urn:cfg:shacl> .
                <urn:cfg:shacl> f:shaclEnabled true .
                <urn:cfg:shacl> f:shapesSource <urn:cfg:shapes-ref> .
                <urn:cfg:shapes-ref> rdf:type f:GraphRef ;
                                     f:graphSource <urn:cfg:shapes-src> .
                <urn:cfg:shapes-src> f:ledger <{model_id}> ;
                                     f:graphSelector <{shapes_graph_iri}> .
            }}
        "
        ))
        .execute()
        .await
        .expect("seed D cross-ledger SHACL config");
    let data = r1.ledger;

    // ex:alice references ex:illinois — a value-set member defined ONLY in M.
    // Membership must resolve against M's live vocabulary.
    let ok = fluree
        .insert(
            data,
            &json!({
                "@context": {"ex": "http://example.org/ns/"},
                "@id": "ex:alice",
                "@type": "ex:Person",
                "ex:homeState": {"@id": "ex:illinois"}
            }),
        )
        .await
        .expect("value-set member defined in the model ledger must satisfy sh:class");
    let data = ok.ledger;

    // ex:bob references ex:atlantis — not a member of the value-set in M.
    let err = fluree
        .insert(
            data,
            &json!({
                "@context": {"ex": "http://example.org/ns/"},
                "@id": "ex:bob",
                "@type": "ex:Person",
                "ex:homeState": {"@id": "ex:atlantis"}
            }),
        )
        .await
        .expect_err("non-member value must be rejected by cross-ledger sh:class");
    assert!(
        matches!(
            err,
            fluree_db_api::ApiError::Transact(fluree_db_transact::TransactError::ShaclViolation(_))
        ),
        "expected ShaclViolation for non-member, got: {err:?}"
    );
}

/// Cross-ledger RDFS entailment for enforcement: the class hierarchy lives
/// in model ledger M (`f:reasoningDefaults` / `f:schemaSource` with
/// `f:ledger`), while the shape lives locally in D. A Manager-typed record
/// must be governed by D's Employee-targeting shape because
/// `Manager rdfs:subClassOf Employee` is declared in M.
#[tokio::test]
async fn cross_ledger_schema_feeds_shacl_subclass_targeting() {
    let fluree = FlureeBuilder::memory().build_memory();

    let model_id = "test/cross-ledger-schema/model:main";
    let model = genesis_ledger(&fluree, model_id);
    let ontology_graph_iri = "http://example.org/governance/ontology";
    let m_trig = format!(
        r"
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        @prefix ex:   <http://example.org/ns/> .

        GRAPH <{ontology_graph_iri}> {{
            ex:Manager rdfs:subClassOf ex:Employee .
        }}
    "
    );
    fluree
        .stage_owned(model)
        .upsert_turtle(&m_trig)
        .execute()
        .await
        .expect("seed M ontology");

    let data_id = "test/cross-ledger-schema/data:main";
    let data = genesis_ledger(&fluree, data_id);

    // D: local Employee shape + config pointing the schema source at M.
    let config_iri = config_graph_iri(data_id);
    let r1 = fluree
        .stage_owned(data)
        .upsert_turtle(&format!(
            r"
            @prefix f:    <https://ns.flur.ee/db#> .
            @prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
            @prefix sh:   <http://www.w3.org/ns/shacl#> .
            @prefix ex:   <http://example.org/ns/> .

            ex:EmployeeShape rdf:type sh:NodeShape ;
                sh:targetClass ex:Employee ;
                sh:property ex:EmployeeShape-name .
            ex:EmployeeShape-name sh:path ex:name ;
                sh:minCount 1 .

            GRAPH <{config_iri}> {{
                <urn:cfg:main> rdf:type f:LedgerConfig ;
                               f:shaclDefaults <urn:cfg:shacl> ;
                               f:reasoningDefaults <urn:cfg:reason> .
                <urn:cfg:shacl> f:shaclEnabled true .
                <urn:cfg:reason> f:schemaSource <urn:cfg:schema-ref> .
                <urn:cfg:schema-ref> rdf:type f:GraphRef ;
                                     f:graphSource <urn:cfg:schema-src> .
                <urn:cfg:schema-src> f:ledger <{model_id}> ;
                                     f:graphSelector <{ontology_graph_iri}> .
            }}
        "
        ))
        .execute()
        .await
        .expect("seed D shape + cross-ledger schema config");
    let data = r1.ledger;

    // Manager without ex:name → rejected via M's subclass edge.
    let err = fluree
        .insert(
            data.clone(),
            &json!({
                "@context": {"ex": "http://example.org/ns/"},
                "@id": "ex:grace",
                "@type": "ex:Manager"
            }),
        )
        .await
        .expect_err("Manager must be governed by the Employee shape via M's hierarchy");
    assert!(
        matches!(
            err,
            fluree_db_api::ApiError::Transact(fluree_db_transact::TransactError::ShaclViolation(_))
        ),
        "expected ShaclViolation, got: {err:?}"
    );

    // Conforming Manager passes.
    fluree
        .insert(
            data,
            &json!({
                "@context": {"ex": "http://example.org/ns/"},
                "@id": "ex:hana",
                "@type": "ex:Manager",
                "ex:name": "Hana"
            }),
        )
        .await
        .expect("conforming Manager must pass");
}
