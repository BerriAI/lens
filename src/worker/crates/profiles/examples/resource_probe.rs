use std::{path::PathBuf, time::Instant};

use lens_profiles::Classifier;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cache = std::env::var_os("LENS_PROFILE_MODEL_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("lens-profile-model-cache"));
    let requests: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "12".into())
        .parse()?;
    let mut classifier = Classifier::new(cache);
    let mut clusters = Vec::new();
    let started = Instant::now();
    let tasks = [
        "Fix the failing unit tests in the authentication service",
        "Repair broken authentication tests",
        "Generate a workflow diagram for purchase approval",
        "Draw the purchase approval process",
        "Summarize meeting notes and list next actions",
        "Extract action items from meeting minutes",
    ];
    let mut memberships = Vec::new();
    for index in 0..requests {
        let task = format!("{} for issue {index}", tasks[index % tasks.len()]);
        let assigned = classifier.classify(&task, &clusters)?;
        if let Some(cluster) = assigned.new_cluster {
            clusters.push(cluster);
        }
        if index < tasks.len() {
            memberships.push(
                clusters
                    .iter()
                    .position(|cluster| cluster.id == assigned.cluster_id),
            );
        }
    }
    println!(
        "requests={requests} clusters={} elapsed_ms={}",
        clusters.len(),
        started.elapsed().as_millis()
    );
    println!("first_six_memberships={memberships:?}");
    Ok(())
}
