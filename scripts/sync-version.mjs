#!/usr/bin/env node
// Copy package.json's version to every other file that carries it.
//
// `changeset version` bumps package.json and writes CHANGELOG.md, but knows
// nothing about the Tauri and Cargo manifests or the version string shown in
// Settings. Run after it (see the `changeset:version` script) so a release
// never ships with the app, the updater and the CLI disagreeing on a version.
import fs from 'node:fs';
import path from 'node:path';

const root = process.cwd();

const packagePath = path.join(root, 'package.json');
const tauriConfPath = path.join(root, 'src-tauri', 'tauri.conf.json');
const cargoTomlPath = path.join(root, 'src-tauri', 'Cargo.toml');
const cargoLockPath = path.join(root, 'src-tauri', 'Cargo.lock');
const enI18nPath = path.join(root, 'src', 'i18n', 'en.json');

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, 'utf8'));
}

function writeJson(filePath, value) {
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`);
}

function updateSettingsVersion(i18nObj, version, fileLabel) {
  if (!i18nObj.settings || typeof i18nObj.settings.version !== 'string') {
    throw new Error(`Missing settings.version in ${fileLabel}`);
  }
  i18nObj.settings.version = i18nObj.settings.version.replace(/\d+\.\d+\.\d+/, version);
}

function updateCargoPackageVersion(cargoToml, version) {
  const packageStart = cargoToml.indexOf('[package]');
  if (packageStart === -1) {
    throw new Error('Missing [package] in src-tauri/Cargo.toml');
  }
  const nextSection = cargoToml.indexOf('\n[', packageStart + '[package]'.length);
  const packageEnd = nextSection === -1 ? cargoToml.length : nextSection;
  const packageSection = cargoToml.slice(packageStart, packageEnd);
  if (!/^version = "[^"]+"$/m.test(packageSection)) {
    throw new Error('Missing package version in src-tauri/Cargo.toml');
  }
  const updatedSection = packageSection.replace(
    /^version = "[^"]+"$/m,
    `version = "${version}"`,
  );
  return `${cargoToml.slice(0, packageStart)}${updatedSection}${cargoToml.slice(packageEnd)}`;
}

function updateCargoLockVersion(cargoLock, version) {
  const packagePattern = /(\[\[package\]\]\nname = "skills-manager"\nversion = ")[^"]+("\n)/;
  if (!packagePattern.test(cargoLock)) {
    throw new Error('Missing skills-manager package entry in src-tauri/Cargo.lock');
  }
  return cargoLock.replace(
    packagePattern,
    (_match, prefix, suffix) => `${prefix}${version}${suffix}`,
  );
}

function main() {
  const { version } = readJson(packagePath);
  if (!/^\d+\.\d+\.\d+$/.test(version)) {
    throw new Error(`package.json version is not SemVer: ${version}`);
  }

  const tauriConf = readJson(tauriConfPath);
  tauriConf.version = version;
  const cargoToml = updateCargoPackageVersion(fs.readFileSync(cargoTomlPath, 'utf8'), version);
  const cargoLock = updateCargoLockVersion(fs.readFileSync(cargoLockPath, 'utf8'), version);
  const en = readJson(enI18nPath);
  updateSettingsVersion(en, version, 'src/i18n/en.json');

  writeJson(tauriConfPath, tauriConf);
  fs.writeFileSync(cargoTomlPath, cargoToml);
  fs.writeFileSync(cargoLockPath, cargoLock);
  writeJson(enI18nPath, en);

  console.log(`Synced version ${version} to tauri.conf.json, Cargo.toml, Cargo.lock and en.json`);
}

main();
