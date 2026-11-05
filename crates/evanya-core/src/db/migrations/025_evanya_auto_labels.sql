-- The app was renamed from Flectar Mail to Evanya. Auto labels are local
-- only, so their stable keywords, and the sender cache that stores them,
-- simply take the new names.
UPDATE labels
SET keyword = 'EvanyaAuto' || substr(keyword, length('FlectarMailAuto') + 1)
WHERE keyword LIKE 'FlectarMailAuto%';

UPDATE route_cache
SET route_key = 'EvanyaAuto' || substr(route_key, length('FlectarMailAuto') + 1)
WHERE route_key LIKE 'FlectarMailAuto%';
