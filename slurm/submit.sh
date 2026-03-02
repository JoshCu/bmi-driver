#!/usr/bin/env bash
# SLURM job-array submission script for bmi-driver
#
# Usage:
#   sbatch --array=0-<N_NODES-1> slurm/submit.sh <data_dir> <total_locations>
#
# Example — 50 000 locations across 16 nodes (64 cores each, ~1 000 cores total):
#   sbatch --array=0-15 slurm/submit.sh /shared/data/my_basin 50000
#
# Each array element handles an equal-sized slice of the location list.
# Results are written to <data_dir>/outputs/bmi-driver/<location_id>.csv on the
# shared filesystem — no post-merge step is required.
#
# Adjust the SBATCH directives below to match your cluster's partition names,
# memory limits, and wall-clock requirements.

#SBATCH --job-name=bmi-driver
#SBATCH --nodes=1
#SBATCH --ntasks=1
# Use all cores on the node for intra-node parallelism:
#SBATCH --cpus-per-task=64
#SBATCH --time=08:00:00
#SBATCH --output=logs/bmi-driver_%A_%a.out
#SBATCH --error=logs/bmi-driver_%A_%a.err

set -euo pipefail

DATA_DIR="${1:?Usage: sbatch --array=0-N slurm/submit.sh <data_dir> <total_locations>}"
TOTAL_LOCATIONS="${2:?Usage: sbatch --array=0-N slurm/submit.sh <data_dir> <total_locations>}"

# Number of array elements = SLURM_ARRAY_TASK_MAX - SLURM_ARRAY_TASK_MIN + 1
# (works whether --array=0-15 or --array=1-16, etc.)
N_NODES=$(( SLURM_ARRAY_TASK_MAX - SLURM_ARRAY_TASK_MIN + 1 ))
TASK_INDEX=$(( SLURM_ARRAY_TASK_ID - SLURM_ARRAY_TASK_MIN ))

CHUNK=$(( (TOTAL_LOCATIONS + N_NODES - 1) / N_NODES ))
NODE_START=$(( TASK_INDEX * CHUNK ))

mkdir -p logs

echo "Node ${SLURM_ARRAY_TASK_ID}: locations ${NODE_START} – $((NODE_START + CHUNK - 1)) of ${TOTAL_LOCATIONS}"

bmi-driver "${DATA_DIR}" \
    --node-start "${NODE_START}" \
    --node-count "${CHUNK}" \
    -j "${SLURM_CPUS_PER_TASK}" \
    --progress none
