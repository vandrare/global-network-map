# Data and attribution

## Graphical country map

Made with [Natural Earth](https://www.naturalearthdata.com/), public-domain map data at 1:50m scale.
Source: [ne_50m_admin_0_countries.geojson](https://github.com/nvkelso/natural-earth-vector/blob/master/geojson/ne_50m_admin_0_countries.geojson).
The bundled `world-50m.json` retains administrative names and polygon rings, including holes, with coordinates rounded to four decimal places. Downloaded September 24, 2026. The application triangulates the polygons locally; it uses no map tile service.

## World coastline

Made with [Natural Earth](https://www.naturalearthdata.com/), public-domain map data.
Source: [ne_110m_coastline.geojson](https://github.com/nvkelso/natural-earth-vector/blob/master/geojson/ne_110m_coastline.geojson).
The bundled coastline extracts line coordinates and rounds them to three decimal places. Downloaded September 23, 2026.

## Offline city search and country/continent mapping

Data from [GeoNames](https://www.geonames.org/), licensed under [Creative Commons Attribution 4.0](https://creativecommons.org/licenses/by/4.0/).
Sources: [cities15000.zip](https://download.geonames.org/export/dump/cities15000.zip) and [countryInfo.txt](https://download.geonames.org/export/dump/countryInfo.txt).
The bundled extracts retain city/native and ASCII names, country codes, coordinates, first-level administrative codes, country names, and continent codes. City rows are sorted by population. Downloaded September 23, 2026.

## IP-to-city lookups

[IP Geolocation by DB-IP](https://db-ip.com/), DB-IP Lite, licensed under [Creative Commons Attribution 4.0](https://creativecommons.org/licenses/by/4.0/).
The optional updater downloads MMDB conversions distributed by [sapics/ip-location-db](https://github.com/sapics/ip-location-db).
These databases are separate runtime downloads and are not embedded in the executable. The application decodes records locally and maps country codes to continents using GeoNames. No accuracy guarantee is implied by these sources.

An alternative database provided with `--database` remains subject to that database's own license.

## Reference image

`global-network-map.png` was supplied by the user as a visual reference. It is not embedded in or redistributed by the executable. Its rights remain with its original owner.
