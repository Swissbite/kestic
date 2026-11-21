// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Row definitions and `QAbstractListModel` plumbing shared by all list models.
//!
//! A model lists its row fields once (`model_row!`): that one list is the row struct, the QML
//! role names and the role values. The role number is the position in the list, so the first
//! field is role 0 (the row id) and QML may read `model.data(index, 0)`.

use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QMap, QMapPair_QString_QVariant, QModelIndex,
    QString, QVariant,
};

/// A row field as a QML value.
pub(super) trait ToVariant {
    fn to_variant(&self) -> QVariant;
}

impl ToVariant for String {
    fn to_variant(&self) -> QVariant {
        QVariant::from(&QString::from(self))
    }
}

impl ToVariant for bool {
    fn to_variant(&self) -> QVariant {
        QVariant::from(self)
    }
}

impl ToVariant for i32 {
    fn to_variant(&self) -> QVariant {
        QVariant::from(self)
    }
}

impl ToVariant for f64 {
    fn to_variant(&self) -> QVariant {
        QVariant::from(self)
    }
}

/// One row of a list model; implemented by `model_row!`.
pub(super) trait ModelRow {
    /// QML role names; the position is the role number.
    const ROLES: &'static [&'static str];

    /// Value of the role at `index` in `ROLES`.
    fn value(&self, index: usize) -> QVariant;

    /// All role values in the order of `ROLES`.
    fn values(&self) -> Vec<QVariant>;
}

/// Defines a row struct and its `ModelRow` implementation from one field list; the optional
/// `hidden` fields are part of the row but have no QML role.
macro_rules! model_row {
    ($row:ident { $($field:ident : $ty:ty => $role:literal,)+ } $(hidden { $($plain:ident : $plain_ty:ty,)+ })?) => {
        #[derive(Clone, Default, PartialEq)]
        struct $row {
            $($field: $ty,)+
            $($($plain: $plain_ty,)+)?
        }

        impl $crate::models::rows::ModelRow for $row {
            const ROLES: &'static [&'static str] = &[$($role),+];

            fn value(&self, index: usize) -> QVariant {
                let values: &[&dyn $crate::models::rows::ToVariant] = &[$(&self.$field),+];
                values
                    .get(index)
                    .map_or_else(QVariant::default, |value| value.to_variant())
            }

            fn values(&self) -> Vec<QVariant> {
                vec![$($crate::models::rows::ToVariant::to_variant(&self.$field)),+]
            }
        }
    };
}

pub(super) fn role_names<R: ModelRow>() -> QHash<QHashPair_i32_QByteArray> {
    let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
    for (role, name) in (0..).zip(R::ROLES) {
        roles.insert(role, QByteArray::from(*name));
    }
    roles
}

pub(super) fn row_data<R: ModelRow>(rows: &[R], index: &QModelIndex, role: i32) -> QVariant {
    let row = usize::try_from(index.row()).ok().and_then(|i| rows.get(i));
    let role = usize::try_from(role).ok();
    match (row, role) {
        (Some(row), Some(role)) => row.value(role),
        _ => QVariant::default(),
    }
}

/// All roles of one row as a QML object; an invalid variant when there is no such row. This is
/// the Rust side of the QML function `rowMap(id)` of the bridge (`models.rs`).
pub(super) fn row_map<R: ModelRow>(row: Option<&R>) -> QVariant {
    let Some(row) = row else {
        return QVariant::default();
    };
    let mut map = QMap::<QMapPair_QString_QVariant>::default();
    for (name, value) in R::ROLES.iter().zip(row.values()) {
        map.insert_clone(&QString::from(*name), &value);
    }
    QVariant::from(&map)
}

/// Position of the first row for which `is_row` holds, or -1. This is the Rust side of the QML
/// function `rowOf(id)` of the bridge (`models.rs`).
pub(super) fn row_of<R>(rows: &[R], is_row: impl Fn(&R) -> bool) -> i32 {
    rows.iter()
        .position(is_row)
        .and_then(|row| i32::try_from(row).ok())
        .unwrap_or(-1)
}

/// The id of the row at `row` as a string, empty if there is no such row. This is the Rust
/// side of the QML function `idAt(row)` of the bridge (`models.rs`).
pub(super) fn id_at<R>(rows: &[R], row: i32, id: impl Fn(&R) -> &str) -> QString {
    let row = usize::try_from(row).ok().and_then(|i| rows.get(i));
    QString::from(row.map_or("", id))
}

pub(super) fn row_count<R>(rows: &[R]) -> i32 {
    i32::try_from(rows.len()).unwrap_or(i32::MAX)
}

/// Implements `sync`, `row_count`, `role_names` and `data` of a list model on its
/// `qobject::<Model>`; `compute_rows` of the Rust struct produces the rows.
macro_rules! list_model {
    (
        $model:ident, $row:ty,
        same: |$a:ident, $b:ident| $same:expr,
        begin_reset: $begin:ident,
        end_reset: $end:ident,
        index: $index:ident,
        changed: $changed:ident $(,)?
    ) => {
        impl $crate::models::qobject::$model {
            pub fn sync(mut self: ::std::pin::Pin<&mut Self>) {
                let new_rows = self.as_ref().compute_rows();
                match $crate::models::plan_model_sync(&self.rows, new_rows, |$a, $b| $same) {
                    $crate::models::SyncPlan::NoChange => {}
                    $crate::models::SyncPlan::InPlace {
                        first,
                        last,
                        new_rows,
                    } => {
                        self.as_mut().rust_mut().rows = new_rows;
                        let parent = QModelIndex::default();
                        let top = self.as_ref().$index(first, 0, &parent);
                        let bottom = self.as_ref().$index(last, 0, &parent);
                        self.as_mut().$changed(&top, &bottom);
                    }
                    $crate::models::SyncPlan::Reset(new_rows) => {
                        self.as_mut().$begin();
                        self.as_mut().rust_mut().rows = new_rows;
                        self.as_mut().$end();
                    }
                }
            }

            pub fn row_count(&self, _parent: &QModelIndex) -> i32 {
                $crate::models::rows::row_count(&self.rows)
            }

            pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
                $crate::models::rows::role_names::<$row>()
            }

            pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
                $crate::models::rows::row_data(&self.rows, index, role)
            }
        }
    };
}
