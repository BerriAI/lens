CREATE VIEW IF NOT EXISTS {database}.lens_task_profile_assignment_rows AS
SELECT TeamId, ApiKeyHash, TraceId, TaxonomyId, TaxonomyVersion, TaskId, Outcome, Attempts, StartTs, ComputedAt
FROM {database}.lens_task_profile_assignments
UNION ALL
SELECT TeamId, ApiKeyHash, TraceId, TaxonomyId, TaxonomyVersion, TaskId, Outcome, Attempts, StartTs, ComputedAt
FROM {database}.lens_task_profile_assignments_monthly
