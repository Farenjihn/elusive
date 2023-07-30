#!/bin/bash

docker build -t quay.io/farenjihn/elusive/builder:latest .
docker push quay.io/farenjihn/elusive/builder:latest
