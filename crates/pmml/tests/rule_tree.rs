//! Clementine 11 inline `RuleTreeModel` maps onto the same `TreeModel` the PMML path scores.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use dms_pmml::{Model, Pmml, Predicate, Value};

fn rule_tree_xml() -> String {
    r#"<model class="com.spss.clementine.component.model.RuleTreeModel">
  <name>y</name>
  <algorithmName>C&amp;R Tree</algorithmName>
  <inputs>
    <input>
      <name>x</name>
      <type><storage>real</storage><measure>range</measure></type>
    </input>
    <input>
      <name>flag</name>
      <type>
        <storage>integer</storage>
        <measure>flag</measure>
        <values><value type="I">0</value><value type="I">1</value></values>
      </type>
    </input>
  </inputs>
  <outputs>
    <output class="com.spss.clementine.component.Model$Output">
      <name>$R-y</name>
      <type><storage>real</storage><measure>range</measure></type>
      <modelFieldInfo role="predictedValue" targetField="y"/>
    </output>
  </outputs>
  <root class="com.spss.clementine.component.model.Rule">
    <children>
      <child class="com.spss.clementine.component.model.DecisionRuleNode">
        <children>
          <child class="com.spss.clementine.component.model.DecisionRuleNode">
            <children/>
            <result type="D">1.0</result>
            <recordCount>10.0</recordCount>
            <id>-1</id>
            <biggestChild>-1</biggestChild>
            <condition class="com.spss.clementine.component.model.TrueCondition"/>
          </child>
        </children>
        <result type="D">1.0</result>
        <recordCount>10.0</recordCount>
        <id>1</id>
        <biggestChild>-1</biggestChild>
        <condition class="com.spss.clementine.component.model.LogicalCondition">
          <children>
            <child class="com.spss.clementine.component.model.SimpleCondition">
              <field>x</field>
              <op>lessOrEqual</op>
              <value type="D">0.5</value>
            </child>
            <child class="com.spss.clementine.component.model.SimpleSetCondition">
              <op>in</op>
              <values><value type="I">0</value></values>
              <field>flag</field>
              <field>flag</field>
            </child>
          </children>
          <op>surrogate</op>
        </condition>
      </child>
      <child class="com.spss.clementine.component.model.DecisionRuleNode">
        <children>
          <child class="com.spss.clementine.component.model.DecisionRuleNode">
            <children/>
            <result type="D">2.0</result>
            <recordCount>10.0</recordCount>
            <id>-1</id>
            <biggestChild>-1</biggestChild>
            <condition class="com.spss.clementine.component.model.TrueCondition"/>
          </child>
        </children>
        <result type="D">2.0</result>
        <recordCount>10.0</recordCount>
        <id>2</id>
        <biggestChild>-1</biggestChild>
        <condition class="com.spss.clementine.component.model.LogicalCondition">
          <children>
            <child class="com.spss.clementine.component.model.SimpleCondition">
              <field>x</field>
              <op>greaterThan</op>
              <value type="D">0.5</value>
            </child>
            <child class="com.spss.clementine.component.model.SimpleSetCondition">
              <op>in</op>
              <values><value type="I">1</value></values>
              <field>flag</field>
              <field>flag</field>
            </child>
          </children>
          <op>surrogate</op>
        </condition>
      </child>
    </children>
    <result type="D">1.5</result>
    <recordCount>20.0</recordCount>
    <id>0</id>
    <biggestChild>0</biggestChild>
  </root>
</model>"#
        .to_string()
}

fn pmml_equivalent() -> String {
    r#"<PMML version="4.3">
  <DataDictionary>
    <DataField name="x" optype="continuous" dataType="double"/>
    <DataField name="flag" optype="categorical" dataType="integer">
      <Value value="0"/><Value value="1"/>
    </DataField>
    <DataField name="y" optype="continuous" dataType="double"/>
  </DataDictionary>
  <TreeModel functionName="regression" algorithmName="C&amp;R Tree" missingValueStrategy="none" noTrueChildStrategy="returnNullPrediction">
    <MiningSchema>
      <MiningField name="x"/>
      <MiningField name="flag"/>
      <MiningField name="y" usageType="predicted"/>
    </MiningSchema>
    <Node id="0" score="1.5" recordCount="20" defaultChild="1">
      <True/>
      <Node id="1" score="1.0" recordCount="10">
        <CompoundPredicate booleanOperator="surrogate">
          <SimplePredicate field="x" operator="lessOrEqual" value="0.5"/>
          <SimpleSetPredicate field="flag" booleanOperator="isIn"><Array n="1" type="int">0</Array></SimpleSetPredicate>
          <True/>
        </CompoundPredicate>
        <Node score="1.0" recordCount="10"><True/></Node>
      </Node>
      <Node id="2" score="2.0" recordCount="10">
        <CompoundPredicate booleanOperator="surrogate">
          <SimplePredicate field="x" operator="greaterThan" value="0.5"/>
          <SimpleSetPredicate field="flag" booleanOperator="isIn"><Array n="1" type="int">1</Array></SimpleSetPredicate>
          <False/>
        </CompoundPredicate>
        <Node score="2.0" recordCount="10"><True/></Node>
      </Node>
    </Node>
  </TreeModel>
</PMML>"#
        .to_string()
}

fn score(pmml: &Pmml, x: Option<f64>, flag: Option<i64>) -> Value {
    let scorer = pmml.bind(&["x", "flag"]).unwrap();
    let record = [
        x.map_or(Value::Missing, Value::Number),
        flag.map_or(Value::Missing, |n| Value::Number(n as f64)),
    ];
    scorer.score(&record).unwrap().value
}

#[test]
fn rule_tree_scores_like_equivalent_pmml() {
    let tree = Pmml::parse_rule_tree(&rule_tree_xml()).unwrap();
    let Model::Tree(model) = &tree.model else {
        panic!("expected TreeModel");
    };
    assert_eq!(model.header.algorithm.as_deref(), Some("C&R Tree"));
    assert!(matches!(
        model.root.children[0].predicate,
        Predicate::Compound { .. }
    ));
    let pmml = Pmml::parse(&pmml_equivalent()).unwrap();
    for (x, flag) in [
        (Some(0.0), Some(0)),
        (Some(1.0), Some(1)),
        (None, Some(0)),
        (None, Some(1)),
        (None, None),
        (Some(0.0), None),
        (Some(1.0), None),
    ] {
        assert_eq!(
            score(&tree, x, flag),
            score(&pmml, x, flag),
            "x={x:?} flag={flag:?}"
        );
    }
    assert_eq!(score(&tree, Some(0.0), Some(0)), Value::Number(1.0));
    assert_eq!(score(&tree, Some(1.0), Some(1)), Value::Number(2.0));
    // Missing x: surrogate uses flag; missing both: biggestChild 0 (score 1.0).
    assert_eq!(score(&tree, None, Some(1)), Value::Number(2.0));
    assert_eq!(score(&tree, None, None), Value::Number(1.0));
}

#[test]
fn rejects_a_document_that_is_not_a_rule_tree() {
    assert!(Pmml::parse_rule_tree("<PMML version=\"4.3\"/>").is_err());
    assert!(Pmml::parse_rule_tree("<model class=\"Other\"/>").is_err());
}
