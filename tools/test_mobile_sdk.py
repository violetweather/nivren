#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

swift = (ROOT / "sdk/mobile/ios/NivrenMobile.swift").read_text(encoding="utf-8")
kotlin = (ROOT / "sdk/mobile/android/NivrenMobile.kt").read_text(encoding="utf-8")
jni = (ROOT / "sdk/mobile/android/nivren_mobile_jni.c").read_text(encoding="utf-8")

assert "nivren_abi_version() >= 3" in swift
assert "defer { nivren_buffer_free(buffer) }" in swift
assert "Array(source.utf8)" in swift
assert "source.encodeToByteArray()" in kotlin
assert "external fun invoke(source: ByteArray" in kotlin
assert "NIVREN_MOBILE_MAXIMUM" in jni
assert jni.count("nivren_buffer_free(result);") >= 3
assert "GetByteArrayRegion" in jni and "SetByteArrayRegion" in jni
assert "GetStringUTFChars" not in jni

print("mobile SDK ownership and UTF-8 contracts: ok")
