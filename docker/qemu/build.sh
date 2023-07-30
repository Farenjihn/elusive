#!/bin/bash

docker build -t quay.io/farenjihn/elusive/qemu:latest .
docker push quay.io/farenjihn/elusive/qemu:latest
