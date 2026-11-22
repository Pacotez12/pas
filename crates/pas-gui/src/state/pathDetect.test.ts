import { describe, expect, it } from "vitest";
import {
  extractFirstDiffLine,
  findPathMatchesInLine,
  tokenizeText,
} from "./pathDetect";

describe("findPathMatchesInLine", () => {
  it("detecta ruta relativa con número de línea", () => {
    const matches = findPathMatchesInLine("Error en lib/a.dart:42 al compilar");
    expect(matches).toHaveLength(1);
    expect(matches[0]).toMatchObject({
      path: "lib/a.dart",
      line: 42,
      col: undefined,
      raw: "lib/a.dart:42",
    });
    expect("Error en lib/a.dart:42 al compilar".slice(matches[0].start, matches[0].end)).toBe(
      "lib/a.dart:42",
    );
  });

  it("detecta ruta con línea y columna", () => {
    const line = "  test/export_test.dart:42:7  Expected: 3";
    const matches = findPathMatchesInLine(line);
    expect(matches).toHaveLength(1);
    expect(matches[0]).toMatchObject({
      path: "test/export_test.dart",
      line: 42,
      col: 7,
      raw: "test/export_test.dart:42:7",
    });
  });

  it("detecta rutas con espacios entre comillas dobles o simples", () => {
    const line = 'Editó "mi carpeta/archivo con espacios.ts:18" y \'otra ruta/test.dart\'';
    const matches = findPathMatchesInLine(line);
    expect(matches).toHaveLength(2);
    expect(matches[0]).toMatchObject({
      path: "mi carpeta/archivo con espacios.ts",
      line: 18,
    });
    expect(matches[1]).toMatchObject({
      path: "otra ruta/test.dart",
      line: undefined,
    });
  });

  it("detecta rutas absolutas y con tilde", () => {
    const line = "Ver /usr/local/src/main.rs:9 y ~/Proyectos/pas/Cargo.toml";
    const matches = findPathMatchesInLine(line);
    expect(matches).toHaveLength(2);
    expect(matches[0]).toMatchObject({
      path: "/usr/local/src/main.rs",
      line: 9,
    });
    expect(matches[1]).toMatchObject({
      path: "~/Proyectos/pas/Cargo.toml",
      line: undefined,
    });
  });

  it("no confunde URLs http(s) con rutas de archivo", () => {
    const line = "Docs: https://docs.flutter.dev/testing y http://localhost:3000/api/v1";
    const matches = findPathMatchesInLine(line);
    expect(matches).toHaveLength(0);
  });

  it("descarta puntuación final y falsos positivos como horas o versiones", () => {
    const line = "Son las 12:30, versión v2.5.1, archivo src/app.tsx.";
    const matches = findPathMatchesInLine(line);
    expect(matches).toHaveLength(1);
    expect(matches[0]).toMatchObject({
      path: "src/app.tsx",
      line: undefined,
      raw: "src/app.tsx",
    });
  });
});

describe("tokenizeText", () => {
  it("separa texto, rutas de archivo y URLs preservando el orden", () => {
    const tokens = tokenizeText(
      "Listo, el test quedó en test/export_test.dart:15 y mirá https://docs.flutter.dev/testing.",
    );
    expect(tokens).toEqual([
      { kind: "text", text: "Listo, el test quedó en " },
      {
        kind: "path",
        text: "test/export_test.dart:15",
        path: "test/export_test.dart",
        line: 15,
        col: undefined,
      },
      { kind: "text", text: " y mirá " },
      {
        kind: "url",
        text: "https://docs.flutter.dev/testing",
        url: "https://docs.flutter.dev/testing",
      },
      { kind: "text", text: "." },
    ]);
  });

  it("maneja rutas con espacios entre comillas en texto de chat", () => {
    const tokens = tokenizeText('Abrí "src/mi archivo.ts:7" para revisar.');
    expect(tokens).toEqual([
      { kind: "text", text: "Abrí " },
      {
        kind: "path",
        text: '"src/mi archivo.ts:7"',
        path: "src/mi archivo.ts",
        line: 7,
        col: undefined,
      },
      { kind: "text", text: " para revisar." },
    ]);
  });
});

describe("extractFirstDiffLine", () => {
  it("extrae la línea inicial del primer hunk del diff unificado", () => {
    const unified = [
      "--- a/lib/export.dart",
      "+++ b/lib/export.dart",
      "@@ -10,3 +42,5 @@ void exportCsv() {",
      "+  final csv = toCsv(gastos);",
    ].join("\n");
    expect(extractFirstDiffLine(unified)).toBe(42);
  });

  it("devuelve undefined si no hay hunk válido", () => {
    expect(extractFirstDiffLine("sin encabezados")).toBeUndefined();
  });
});
