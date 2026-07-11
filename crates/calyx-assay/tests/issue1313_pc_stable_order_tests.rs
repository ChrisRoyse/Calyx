mod issue1313_support;

use issue1313_support::{
    asymmetric_separator_fixture, canonical_edges, discover, has_edge, name_set, separating_set,
};

#[test]
fn pc_stable_tests_both_endpoint_adjacencies_independent_of_variable_order() {
    let data = asymmetric_separator_fixture(256);
    let forward = discover(&data, &["i", "a", "w", "j"], 2);
    let reversed = discover(&data, &["j", "a", "w", "i"], 2);

    assert!(!has_edge(&forward, "i", "j"), "{forward:#?}");
    assert!(!has_edge(&reversed, "i", "j"), "{reversed:#?}");
    assert_eq!(canonical_edges(&forward), canonical_edges(&reversed));
    assert!(has_edge(&forward, "i", "a"));
    assert!(has_edge(&reversed, "i", "a"));
    assert_eq!(
        separating_set(&forward, "i", "j"),
        Some(name_set(&["a", "w"]))
    );
    assert_eq!(
        separating_set(&reversed, "i", "j"),
        Some(name_set(&["a", "w"]))
    );
}
