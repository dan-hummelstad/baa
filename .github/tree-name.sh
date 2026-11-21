#!/usr/bin/env bash
# The tree to go on from among a chromium.yml run's unexpired artifacts (a JSON array on stdin,
# $TREE = chromium-tree-xcode<version>): the finished tree if the run has one, else the tree of
# the last phase that ended. Prints its artifact name, or nothing.
set -eu
jq -r '
  (env.TREE + "-phase") as $p
  | [.[].name] as $names
  | if $names | index(env.TREE) then env.TREE
    else [$names[] | select(startswith($p)) | ltrimstr($p) | select(test("^[0-9]+$")) | tonumber]
      | max // empty | $p + tostring
    end'
