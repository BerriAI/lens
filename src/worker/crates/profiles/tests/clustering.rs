use lens_profiles::{
    Classifier, DIMENSIONS, MAX_CLUSTERS, MAX_TASK_BYTES, TaskCluster, assign, task_description,
};
use rstest::{fixture, rstest};

#[fixture]
fn coding_vector() -> Vec<f32> {
    let mut vector = vec![0.0; DIMENSIONS];
    vector[0] = 1.0;
    vector
}

#[fixture]
fn coding_cluster(coding_vector: Vec<f32>) -> TaskCluster {
    assign("Fix failing unit tests", coding_vector, &[])
        .unwrap()
        .new_cluster
        .unwrap()
}

#[rstest]
fn semantically_similar_tasks_reuse_a_stable_cluster(
    coding_vector: Vec<f32>,
    coding_cluster: TaskCluster,
) {
    let result = assign(
        "Repair the broken test suite",
        coding_vector,
        std::slice::from_ref(&coding_cluster),
    )
    .unwrap();
    assert_eq!(result.cluster_id, coding_cluster.id);
    assert_eq!(result.label, coding_cluster.label);
    assert!(result.new_cluster.is_none());
}

#[rstest]
fn distinct_tasks_create_a_new_cluster(coding_cluster: TaskCluster) {
    let mut workflow_vector = vec![0.0; DIMENSIONS];
    workflow_vector[1] = 1.0;
    let result = assign(
        "Draw purchase approval workflow",
        workflow_vector,
        std::slice::from_ref(&coding_cluster),
    )
    .unwrap();
    assert_ne!(result.cluster_id, coding_cluster.id);
    assert_eq!(result.new_cluster.unwrap().label, result.label);
}

#[rstest]
fn full_cluster_set_keeps_matching_existing_tasks(
    coding_vector: Vec<f32>,
    coding_cluster: TaskCluster,
) {
    let clusters = vec![coding_cluster.clone(); MAX_CLUSTERS];
    let result = assign("Repair tests", coding_vector, &clusters).unwrap();
    assert_eq!(result.cluster_id, coding_cluster.id);
    assert!(result.new_cluster.is_none());
}

#[rstest]
fn full_cluster_set_does_not_force_unrelated_tasks_into_an_existing_group(
    coding_cluster: TaskCluster,
) {
    let mut vector = vec![0.0; DIMENSIONS];
    vector[1] = 1.0;
    let result = assign("Plan vacation", vector, &vec![coding_cluster; MAX_CLUSTERS]).unwrap();
    assert!(result.cluster_id.is_empty());
    assert!(result.new_cluster.is_none());
}

#[rstest]
#[case::space(" Fix   failing unit tests ")]
#[case::case("FIX FAILING UNIT TESTS")]
fn normalization_preserves_cluster_identity(#[case] input: &str, coding_vector: Vec<f32>) {
    let expected = assign("Fix failing unit tests", coding_vector.clone(), &[]).unwrap();
    let result = assign(input, coding_vector, &[]).unwrap();
    assert_eq!(result.cluster_id, expected.cluster_id);
}

#[rstest]
#[case::ascii("a")]
#[case::multibyte("工作")]
#[case::lowercase_expands("İ")]
fn description_is_bounded_and_unicode_safe(#[case] character: &str) {
    let input = character.repeat(MAX_TASK_BYTES * 4);
    let result = task_description(&input);
    assert!(result.len() <= MAX_TASK_BYTES);
    assert!(!result.is_empty());
    assert!(std::str::from_utf8(result.as_bytes()).is_ok());
}

#[rstest]
#[case::empty(vec![])]
#[case::zero(vec![0.0; DIMENSIONS])]
#[case::nonfinite(vec![f32::NAN; DIMENSIONS])]
#[case::overflow(vec![f32::MAX; DIMENSIONS])]
fn invalid_embeddings_are_rejected(#[case] embedding: Vec<f32>) {
    assert!(assign("A task", embedding, &[]).is_err());
}

#[rstest]
fn persisted_clusters_round_trip_without_changing_assignment(
    coding_vector: Vec<f32>,
    coding_cluster: TaskCluster,
) {
    let serialized = serde_json::to_string(&coding_cluster).unwrap();
    let restored = serde_json::from_str::<TaskCluster>(&serialized).unwrap();
    let result = assign("Repair tests", coding_vector, &[restored]).unwrap();
    assert_eq!(result.cluster_id, coding_cluster.id);
}

#[rstest]
fn excessive_cluster_sets_are_rejected(coding_vector: Vec<f32>, coding_cluster: TaskCluster) {
    assert!(
        assign(
            "Repair tests",
            coding_vector,
            &vec![coding_cluster; MAX_CLUSTERS + 1]
        )
        .is_err()
    );
}

#[rstest]
fn empty_tasks_do_not_create_clusters(coding_vector: Vec<f32>) {
    let result = assign(" \n \t", coding_vector, &[]).unwrap();
    assert!(result.cluster_id.is_empty());
    assert!(result.new_cluster.is_none());
}

#[rstest]
fn unsupported_scripts_do_not_create_misleading_clusters(coding_vector: Vec<f32>) {
    let result = assign("整理會議紀錄並列出下一步行動", coding_vector, &[]).unwrap();
    assert!(result.cluster_id.is_empty());
    assert!(result.new_cluster.is_none());
}

#[rstest]
#[ignore = "downloads the local embedding model on first run"]
fn local_model_groups_paraphrases_and_separates_unrelated_tasks() {
    let cache = std::env::var_os("LENS_PROFILE_MODEL_CACHE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("lens-profile-model-cache"));
    let mut classifier = Classifier::new(cache);
    let coding = classifier
        .classify(
            "Fix the failing unit tests in the authentication service",
            &[],
        )
        .unwrap()
        .new_cluster
        .unwrap();
    let repair = classifier
        .classify(
            "Repair broken authentication tests",
            std::slice::from_ref(&coding),
        )
        .unwrap();
    assert_eq!(repair.cluster_id, coding.id);
    assert!(repair.new_cluster.is_none());
    let workflow = classifier
        .classify(
            "Generate a workflow diagram for purchase approval",
            std::slice::from_ref(&coding),
        )
        .unwrap()
        .new_cluster
        .unwrap();
    assert_ne!(workflow.id, coding.id);
    let draw = classifier
        .classify(
            "Draw the purchase approval process",
            &[coding, workflow.clone()],
        )
        .unwrap();
    assert_eq!(draw.cluster_id, workflow.id);
    assert!(draw.new_cluster.is_none());
}
