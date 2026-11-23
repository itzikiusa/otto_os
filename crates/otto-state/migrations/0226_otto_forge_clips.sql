-- The built-in Otto Forge's argument template gains `--clips {clips}` (the
-- moves `forge animate` makes; Smart mode passes the plan's moves). A row
-- whose template is still exactly the old seed gets the new one; a template
-- the user edited keeps its value. Data only: no schema change. A previous
-- build does not fill {clips}, so after a rollback its animate stage would get
-- the literal "{clips}"; reset the template (clear it in the provider form) then.
UPDATE media_3d_providers
   SET cli_args_json = '["{stage}","--mesh","{mesh}","--image","{image}","--prompt","{prompt}","--output","{output}","--poly-budget","{poly_budget}","--seed","{seed}","--clips","{clips}"]'
 WHERE id = 'otto-forge'
   AND cli_args_json = '["{stage}","--mesh","{mesh}","--image","{image}","--prompt","{prompt}","--output","{output}","--poly-budget","{poly_budget}","--seed","{seed}"]';
