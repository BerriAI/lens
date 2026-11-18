SELECT field, object_key, bytes, sha256
FROM lens_payload_refs FINAL
WHERE team_id = {team_id:String} AND api_key = {api_key:String} AND event_id = {event_id:String}
ORDER BY field
