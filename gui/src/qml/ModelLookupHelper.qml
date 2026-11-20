// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick

QtObject {
    id: lookup

    required property var backupsModel
    required property var destinationsModel

    function findBackupData(backupId) {
        var count = lookup.backupsModel.rowCount(lookup.backupsModel.index(-1, -1));
        for (var i = 0; i < count; ++i) {
            var idx = lookup.backupsModel.index(i, 0);
            if (lookup.backupsModel.data(idx, 0) === backupId) {
                return {
                    backupId: lookup.backupsModel.data(idx, 0),
                    name: lookup.backupsModel.data(idx, 1),
                    sourcePath: lookup.backupsModel.data(idx, 2),
                    sourceKind: lookup.backupsModel.data(idx, 3),
                    destinationId: lookup.backupsModel.data(idx, 4),
                    destinationName: lookup.backupsModel.data(idx, 5),
                    excludesText: lookup.backupsModel.data(idx, 6),
                    excludeCaches: lookup.backupsModel.data(idx, 7),
                    statusText: lookup.backupsModel.data(idx, 8),
                    statusIcon: lookup.backupsModel.data(idx, 9),
                    statusColorKind: lookup.backupsModel.data(idx, 10),
                    lastRunSummary: lookup.backupsModel.data(idx, 11),
                    availabilityReason: lookup.backupsModel.data(idx, 12),
                    available: lookup.backupsModel.data(idx, 13),
                    hasError: lookup.backupsModel.data(idx, 14),
                    isOverdue: lookup.backupsModel.data(idx, 15),
                    isRunning: lookup.backupsModel.data(idx, 16),
                    activeRunId: lookup.backupsModel.data(idx, 17),
                    activePercent: lookup.backupsModel.data(idx, 18),
                    activeProgressText: lookup.backupsModel.data(idx, 19),
                    errorSummary: lookup.backupsModel.data(idx, 20),
                    errorAction: lookup.backupsModel.data(idx, 21),
                    unreadableFilesJson: lookup.backupsModel.data(idx, 22),
                    scheduleSummary: lookup.backupsModel.data(idx, 23),
                    retentionSummary: lookup.backupsModel.data(idx, 24),
                    conditionsSummary: lookup.backupsModel.data(idx, 25),
                    lastRunId: lookup.backupsModel.data(idx, 26),
                    excludesJson: lookup.backupsModel.data(idx, 29),
                    excludeFoldersCount: lookup.backupsModel.data(idx, 30),
                    expertRuleCount: lookup.backupsModel.data(idx, 31),
                    excludeSummary: lookup.backupsModel.data(idx, 32),
                    excludeFilePath: lookup.backupsModel.data(idx, 33),
                    excludeLintJson: lookup.backupsModel.data(idx, 34),
                    retentionJson: lookup.backupsModel.data(idx, 35),
                    retentionPreset: lookup.backupsModel.data(idx, 36),
                    sourceKindLabel: lookup.backupsModel.data(idx, 37),
                    driveLabel: lookup.backupsModel.data(idx, 38),
                    pendingForget: lookup.backupsModel.data(idx, 39),
                    nestedWarningsJson: lookup.backupsModel.data(idx, 40),
                    hasAttentionWarning: lookup.backupsModel.data(idx, 41),
                    scheduleJson: lookup.backupsModel.data(idx, 42),
                    conditionsJson: lookup.backupsModel.data(idx, 43),
                    nextScheduledAtText: lookup.backupsModel.data(idx, 44),
                    isPaused: lookup.backupsModel.data(idx, 45),
                    pausedLine: lookup.backupsModel.data(idx, 46),
                    pendingPrimaryCode: lookup.backupsModel.data(idx, 47),
                    pendingPrimaryReason: lookup.backupsModel.data(idx, 48),
                    pendingReasonsTooltip: lookup.backupsModel.data(idx, 49),
                    missedSlotsCount: lookup.backupsModel.data(idx, 50),
                    overdueThreshold: lookup.backupsModel.data(idx, 51),
                    hasUnmetRunCondition: lookup.backupsModel.data(idx, 52),
                    unmetConditionsText: lookup.backupsModel.data(idx, 53),
                    activeScanning: lookup.backupsModel.data(idx, 54)
                };
            }
        }
        return null;
    }

    function findDestinationData(destId) {
        var count = lookup.destinationsModel.rowCount(lookup.destinationsModel.index(-1, -1));
        for (var i = 0; i < count; ++i) {
            var idx = lookup.destinationsModel.index(i, 0);
            if (lookup.destinationsModel.data(idx, 0) === destId) {
                return {
                    destinationId: lookup.destinationsModel.data(idx, 0),
                    name: lookup.destinationsModel.data(idx, 1),
                    kindName: lookup.destinationsModel.data(idx, 2),
                    kindLabel: lookup.destinationsModel.data(idx, 3),
                    location: lookup.destinationsModel.data(idx, 4),
                    username: lookup.destinationsModel.data(idx, 5),
                    backupCount: lookup.destinationsModel.data(idx, 6),
                    statusText: lookup.destinationsModel.data(idx, 7),
                    statusIcon: lookup.destinationsModel.data(idx, 8),
                    reachable: lookup.destinationsModel.data(idx, 9),
                    passwordRejected: lookup.destinationsModel.data(idx, 10),
                    hasStaleLock: lookup.destinationsModel.data(idx, 11),
                    hasProblem: lookup.destinationsModel.data(idx, 12),
                    detailText: lookup.destinationsModel.data(idx, 13),
                    pendingForgetCount: lookup.destinationsModel.data(idx, 14),
                    maintenanceJson: lookup.destinationsModel.data(idx, 15),
                    pruneSummary: lookup.destinationsModel.data(idx, 16),
                    checkSummary: lookup.destinationsModel.data(idx, 17),
                    nextPruneAtText: lookup.destinationsModel.data(idx, 18),
                    nextCheckAtText: lookup.destinationsModel.data(idx, 19),
                    lastPruneAtText: lookup.destinationsModel.data(idx, 20),
                    lastCheckAtText: lookup.destinationsModel.data(idx, 21),
                    maintenanceRunning: lookup.destinationsModel.data(idx, 22),
                    staleLockCount: lookup.destinationsModel.data(idx, 23),
                    maintenanceStatusText: lookup.destinationsModel.data(idx, 24)
                };
            }
        }
        return null;
    }
}
