//! AssociationModel parsing and rule scoring.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use dms_pmml::{Model, Pmml, Usage, score_rules};
use std::collections::HashSet;

const MODEL: &str = r#"<?xml version="1.0"?>
<PMML version="4.2">
<DataDictionary numberOfFields="2">
<DataField name="eggs" optype="categorical" dataType="string">
<Extension extender="spss" name="flagTypeTrueValue" value="T"/>
<Value value="F"/><Value value="T"/>
</DataField>
<DataField name="milk" optype="categorical" dataType="string">
<Value value="F"/><Value value="T"/>
</DataField>
</DataDictionary>
<AssociationModel functionName="associationRules" algorithmName="Apriori" modelName="m"
 minimumSupport="0.5" minimumConfidence="0.8" numberOfTransactions="4"
 numberOfItems="2" numberOfItemsets="2" numberOfRules="1">
<MiningSchema>
<MiningField name="eggs" usageType="active">
<Extension extender="spss" name="direction" value="BOTH"/>
</MiningField>
<MiningField name="basket" usageType="group"/>
</MiningSchema>
<Item id="1" value="eggs"/>
<Item id="2" value="milk"/>
<Itemset id="1" numberOfItems="1" support="0.5"><ItemRef itemRef="1"/></Itemset>
<Itemset id="2" numberOfItems="1" support="0.75"><ItemRef itemRef="2"/></Itemset>
<AssociationRule id="1" antecedent="1" consequent="2" support="0.5" confidence="1.0" lift="1.333"/>
</AssociationModel>
</PMML>"#;

#[test]
fn parses_items_direction_and_group() {
    let pmml = Pmml::parse(MODEL).unwrap();
    let Model::Association(model) = &pmml.model else {
        panic!("not an association model");
    };
    assert_eq!(model.number_of_transactions, 4);
    assert_eq!(model.items.len(), 2);
    assert_eq!(model.itemset_values("1").unwrap(), vec!["eggs"]);
    assert_eq!(
        model
            .header
            .mining_schema
            .iter()
            .find(|field| field.name == "eggs")
            .and_then(|field| field.direction.as_deref()),
        Some("BOTH")
    );
    assert!(
        model
            .header
            .mining_schema
            .iter()
            .any(|field| field.name == "basket" && field.usage == Usage::Group)
    );
    assert_eq!(
        pmml.data_dictionary
            .iter()
            .find(|field| field.name == "eggs")
            .and_then(|field| field.flag_true_value.as_deref()),
        Some("T")
    );
    let mut present = HashSet::new();
    present.insert("eggs".to_string());
    let hits = score_rules(model, &present, 3, false, true).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].item, "milk");
    assert_eq!(hits[0].rule_id, 1);
    present.insert("milk".to_string());
    assert!(
        score_rules(model, &present, 3, false, true)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn multi_item_consequent_is_an_error() {
    let xml = MODEL.replace(
        r#"<Itemset id="2" numberOfItems="1" support="0.75"><ItemRef itemRef="2"/></Itemset>"#,
        r#"<Itemset id="2" numberOfItems="2" support="0.75"><ItemRef itemRef="1"/><ItemRef itemRef="2"/></Itemset>"#,
    );
    let pmml = Pmml::parse(&xml).unwrap();
    let Model::Association(model) = &pmml.model else {
        panic!("not an association model");
    };
    let mut present = HashSet::new();
    present.insert("eggs".to_string());
    assert!(score_rules(model, &present, 3, false, true).is_err());
}
