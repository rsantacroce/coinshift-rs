#!/bin/bash
#
# Shared environment for scripts in ./docs
#
# PROJECT_ROOT is the base directory where the related repos/data dirs live.
# Override it when running scripts, e.g.:
#   PROJECT_ROOT="$HOME/Projects" ./1_start_mainchain.sh
#

: "${PROJECT_ROOT:=/Users/rob/projects/layertwolabs/}"
echo "PROJECT_ROOT: $PROJECT_ROOT"


