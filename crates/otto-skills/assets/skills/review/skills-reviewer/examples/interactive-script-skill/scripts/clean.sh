#!/usr/bin/env bash
read -p "Delete generated files? " answer
if [ "$answer" = "yes" ]; then
  rm -rf dist build target
fi
