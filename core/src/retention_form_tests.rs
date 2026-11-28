// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

fn policy(json: &str) -> RetentionPolicy {
    serde_json::from_str(json).expect("policy json")
}

fn on(num: i64) -> CountRule {
    CountRule {
        on: true,
        all: false,
        num,
    }
}

fn within_on(num: i64, unit: DurationUnit) -> WithinRule {
    WithinRule {
        on: true,
        num,
        unit,
    }
}

/// AC-M6-34-1 (a) and (c): reading a saved policy gives the fields the editor's QML showed
/// (`applyKeepCountField`, `parseDuration`): rules that are off keep their offered value, a
/// duration is trimmed, a number below 1 counts as 1, and a duration with more than one part or
/// an unknown unit is off.
#[test]
fn ac_m6_34_1_retention_form_reads_policies_like_former_qml() {
    let empty = RetentionForm::from_policy(&policy("{}"));
    assert_eq!(
        empty,
        RetentionForm::default(),
        "(c) empty policy: every rule off"
    );
    assert_eq!(empty.keep_last, count_off(10), "(c) keep_last offers 10");
    assert_eq!(
        empty.keep_hourly,
        count_off(24),
        "(c) keep_hourly offers 24"
    );
    assert_eq!(empty.keep_daily, count_off(7), "(c) keep_daily offers 7");
    assert_eq!(empty.keep_weekly, count_off(4), "(c) keep_weekly offers 4");
    assert_eq!(
        empty.keep_monthly,
        count_off(12),
        "(c) keep_monthly offers 12"
    );
    assert_eq!(empty.keep_yearly, count_off(5), "(c) keep_yearly offers 5");
    assert_eq!(empty.keep_within, within_off(30, DurationUnit::Days));
    assert_eq!(
        empty.keep_within_hourly,
        within_off(48, DurationUnit::Hours)
    );
    assert_eq!(empty.keep_within_daily, within_off(14, DurationUnit::Days));
    assert_eq!(
        empty.keep_within_weekly,
        within_off(6, DurationUnit::Months)
    );
    assert_eq!(
        empty.keep_within_monthly,
        within_off(1, DurationUnit::Years)
    );
    assert_eq!(empty.keep_within_yearly, within_off(5, DurationUnit::Years));

    let long = RetentionForm::from_policy(&RetentionPolicy::long());
    assert_eq!(long.keep_daily, on(7), "(c) long: daily 7");
    assert_eq!(
        long.keep_yearly,
        CountRule {
            on: true,
            all: true,
            num: 5
        },
        "(c) unlimited keeps the offered number"
    );

    let zero = RetentionForm::from_policy(&policy(r#"{"keep_last":0}"#));
    assert_eq!(zero.keep_last, count_off(10), "(c) a count of 0 is off");

    let table: [(&str, WithinRule); 7] = [
        (
            r#"{"keep_within":"30d"}"#,
            within_on(30, DurationUnit::Days),
        ),
        (
            r#"{"keep_within":" 48h "}"#,
            within_on(48, DurationUnit::Hours),
        ),
        (
            r#"{"keep_within":"0m"}"#,
            within_on(1, DurationUnit::Months),
        ),
        (r#"{"keep_within":"2y"}"#, within_on(2, DurationUnit::Years)),
        (
            r#"{"keep_within":"1y6m"}"#,
            within_off(30, DurationUnit::Days),
        ),
        (
            r#"{"keep_within":"30x"}"#,
            within_off(30, DurationUnit::Days),
        ),
        (r#"{"keep_within":"d"}"#, within_off(30, DurationUnit::Days)),
    ];
    for (json, expected) in table {
        assert_eq!(
            RetentionForm::from_policy(&policy(json)).keep_within,
            expected,
            "(c) {json}"
        );
    }
}

/// AC-M6-34-1 (a) and (c): the fields give the policy the QML built (`currentPolicyObj`), with
/// a number below 1 written as 1 and «all» as `unlimited`.
#[test]
fn ac_m6_34_1_retention_form_writes_policies_like_former_qml() {
    let mut form = RetentionForm::default();
    assert_eq!(form.to_policy(), RetentionPolicy::empty(), "(c) no rule");

    form.keep_last = on(0);
    form.keep_yearly = CountRule {
        on: true,
        all: true,
        num: 3,
    };
    form.keep_within_daily = within_on(0, DurationUnit::Days);
    form.keep_within_weekly = WithinRule {
        on: false,
        num: 9,
        unit: DurationUnit::Months,
    };
    let written = form.to_policy();
    assert_eq!(written.keep_last, Some(KeepCount::Count(1)), "(c) 0 → 1");
    assert_eq!(written.keep_yearly, Some(KeepCount::Unlimited), "(c) all");
    assert_eq!(
        written.keep_within_daily.as_deref(),
        Some("1d"),
        "(c) 0d → 1d"
    );
    assert_eq!(written.keep_within_weekly, None, "(c) a rule that is off");
    assert_eq!(
        serde_json::to_string(&written).expect("json"),
        r#"{"keep_last":1,"keep_yearly":"unlimited","keep_within_daily":"1d"}"#,
        "(c) the policy JSON"
    );

    for preset in [
        RetentionPolicy::standard(),
        RetentionPolicy::economical(),
        RetentionPolicy::long(),
    ] {
        assert_eq!(
            RetentionForm::from_policy(&preset).to_policy(),
            preset,
            "(c) a preset survives the editor"
        );
    }
}

/// AC-M6-34-1 (c): a number the page's integer field holds out of range (a saved count above
/// its range wraps to a negative number there) is written as 1, as the QML did.
#[test]
fn ac_m6_34_1_retention_form_takes_numbers_out_of_range() {
    let form = RetentionForm {
        keep_last: on(-1_294_967_296),
        keep_within: within_on(5_000_000_000, DurationUnit::Days),
        ..RetentionForm::default()
    };
    let form: RetentionForm =
        serde_json::from_str(&serde_json::to_string(&form).expect("json")).expect("read back");
    let state = form.state(&RetentionPolicy::empty());
    assert!(state.has_rule, "(c) the rule stays on");
    assert_eq!(
        state.policy.keep_last,
        Some(KeepCount::Count(1)),
        "(c) a negative count is written as 1"
    );
    assert_eq!(
        state.policy.keep_within.as_deref(),
        Some("4294967295d"),
        "(c) a number above the count range is capped"
    );
    let saved = policy(r#"{"keep_last":3000000000}"#);
    assert_eq!(
        RetentionForm::from_policy(&saved).keep_last,
        on(3_000_000_000),
        "(c) a large saved count is read as it is"
    );
}

/// AC-M6-34-1 (a): the preset comes from `RetentionPolicy::preset()`, and the editor counts as
/// changed exactly where the QML's normalised JSON differed from the saved policy.
#[test]
fn ac_m6_34_1_retention_form_state_detects_preset_and_changes() {
    let standard = RetentionForm::from_policy(&RetentionPolicy::standard());
    let state = standard.state(&RetentionPolicy::standard());
    assert_eq!(state.preset, RetentionPreset::Standard, "(a) standard");
    assert!(!state.dirty, "(a) unchanged");
    assert!(state.has_rule, "(a) has a rule");

    let economical = RetentionForm::from_policy(&RetentionPolicy::economical());
    let state = economical.state(&RetentionPolicy::standard());
    assert_eq!(state.preset, RetentionPreset::Economical, "(a) economical");
    assert!(state.dirty, "(a) differs from the saved standard");

    let long = RetentionForm::from_policy(&RetentionPolicy::long());
    assert_eq!(
        long.state(&RetentionPolicy::long()).preset,
        RetentionPreset::Long,
        "(a) long"
    );

    let mut custom = standard;
    custom.keep_daily.num = 8;
    assert_eq!(
        custom.state(&RetentionPolicy::standard()).preset,
        RetentionPreset::Custom,
        "(a) custom"
    );

    let none = RetentionForm::default().state(&RetentionPolicy::empty());
    assert!(!none.has_rule, "(a) no rule on");
    assert!(!none.dirty, "(a) empty equals empty");

    // A saved duration the editor rewrites (trimmed, or dropped because it has two parts)
    // counts as a change, as before.
    let padded = policy(r#"{"keep_within":" 30d "}"#);
    assert!(
        RetentionForm::from_policy(&padded).state(&padded).dirty,
        "(c) a padded duration is rewritten"
    );
    let two_parts = policy(r#"{"keep_within":"1y6m","keep_daily":7}"#);
    assert!(
        RetentionForm::from_policy(&two_parts)
            .state(&two_parts)
            .dirty,
        "(c) a two-part duration is dropped"
    );
}

/// AC-M6-34-1 (a): the form's JSON is what the editor page reads and sends.
#[test]
fn ac_m6_34_1_retention_form_json_shape() {
    let json =
        serde_json::to_value(RetentionForm::from_policy(&RetentionPolicy::long())).expect("json");
    assert_eq!(
        json["keep_yearly"],
        serde_json::json!({"on": true, "all": true, "num": 5}),
        "(a) count rule"
    );
    assert_eq!(
        json["keep_within_weekly"],
        serde_json::json!({"on": false, "num": 6, "unit": "m"}),
        "(a) duration rule"
    );
    let state = serde_json::to_value(
        RetentionForm::from_policy(&RetentionPolicy::long()).state(&RetentionPolicy::standard()),
    )
    .expect("json");
    assert_eq!(state["preset"], "long", "(a) preset name");
    assert_eq!(state["dirty"], true, "(a) dirty");
    assert_eq!(state["has_rule"], true, "(a) has_rule");
}
