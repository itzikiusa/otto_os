-- Keep the confinement posture custom agent providers had before S11-309.
--
-- Until then a provider the `process_sandbox.providers` allow-list (default
-- claude/codex/agy/shell) did not name ran UNCONFINED, so every custom
-- provider in the `providers` setting did. S11-309 confines every provider a
-- person has not exempted, which silently confined those existing providers
-- on upgrade. Record each of them in `exempt_providers` once, so the posture
-- they had is now an explicit, visible choice (Settings → Daemon) — and
-- providers added from now on are confined unless a person exempts them.
--
-- Only rows without an `exempt_providers` key are touched (a person's own
-- list is never rewritten), and only when such custom providers exist. A
-- provider the legacy allow-list named was confined before and stays so.
-- Previous builds read the extra key as before (the S11-309 build honours
-- it, older ones ignore it).
UPDATE settings
SET value_json = json_set(
    value_json,
    '$.exempt_providers',
    json((
        SELECT json_group_array(p.key)
        FROM settings AS prov, json_each(prov.value_json) AS p
        WHERE prov.key = 'providers'
          AND json_valid(prov.value_json)
          AND json_type(prov.value_json) = 'object'
          AND p.key NOT IN ('claude', 'codex', 'agy', 'shell')
          AND p.key NOT IN (
              SELECT l.value FROM json_each(
                  CASE WHEN json_type(settings.value_json, '$.providers') = 'array'
                       THEN json_extract(settings.value_json, '$.providers')
                       ELSE '[]' END
              ) AS l
          )
    ))
)
WHERE key = 'process_sandbox'
  AND json_valid(value_json)
  AND json_type(value_json) = 'object'
  AND json_type(value_json, '$.exempt_providers') IS NULL
  AND EXISTS (
      SELECT 1
      FROM settings AS prov, json_each(prov.value_json) AS p
      WHERE prov.key = 'providers'
        AND json_valid(prov.value_json)
        AND json_type(prov.value_json) = 'object'
        AND p.key NOT IN ('claude', 'codex', 'agy', 'shell')
        AND p.key NOT IN (
            SELECT l.value FROM json_each(
                CASE WHEN json_type(settings.value_json, '$.providers') = 'array'
                     THEN json_extract(settings.value_json, '$.providers')
                     ELSE '[]' END
            ) AS l
        )
  );
