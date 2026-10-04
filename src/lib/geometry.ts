import type { CellValue } from "@/lib/wire";

export const WKT_PREFIX =
  /^(POINT|LINESTRING|POLYGON|MULTIPOINT|MULTILINESTRING|MULTIPOLYGON|GEOMETRYCOLLECTION)\s*\(/i;
export const GEOJSON_PREFIX = /^\s*\{\s*"type"\s*:/;
export const WKB_HEX = /^[0-9a-f]{8,}$/i;

export function detectGeomColumnIndex(columns: string[], rows: CellValue[][]): number {
  const sample = rows.slice(0, 10);
  if (sample.length === 0) return -1;
  for (let ci = 0; ci < columns.length; ci++) {
    const colName = columns[ci].toLowerCase();
    const isGeoName =
      /geom|geometry|geography|location|coordinates|the_geom|wkb_geometry|shape|point|latlng/.test(
        colName,
      );

    let geoCount = 0;
    for (const row of sample) {
      const val = row[ci] ?? "";
      if (
        WKT_PREFIX.test(val) ||
        GEOJSON_PREFIX.test(val) ||
        (WKB_HEX.test(val) && val.length >= 40)
      ) {
        geoCount++;
      }
    }

    if (geoCount >= Math.min(3, sample.length) || (isGeoName && geoCount > 0)) {
      return ci;
    }
  }
  return -1;
}

export function hasGeometryColumn(columns: string[], rows: CellValue[][]): boolean {
  return detectGeomColumnIndex(columns, rows) >= 0;
}
