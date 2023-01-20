//! A crate containing various derive macros for `data_model`
#![allow(clippy::std_instead_of_core)]

use proc_macro::TokenStream;
use syn::parse_macro_input;

mod filter;
mod has_origin;
mod id;
mod partially_tagged;

/// Derive macro for `Identifiable` trait which also automatically implements [`Ord`], [`Eq`],
/// and [`Hash`] for the annotated struct by delegating to it's identifier field. Identifier
/// field for the struct can be selected by annotating the desired field with `#[id]` or
/// `#[id(transparent)]`. The use of `transparent` assumes that the field is also `Identifiable`,
/// and the macro takes the field identifier of the annotated structure. In the absence
/// of any helper attribute, the macro uses the field named `id` if there is such a field.
/// Otherwise, the macro expansion fails.
///
/// The macro should never be used on structs that aren't uniquely identifiable
///
/// # Examples
///
/// The common use-case:
///
/// ```rust
/// use iroha_data_model_derive::IdOrdEqHash;
/// use iroha_data_model::Identifiable;
///
/// #[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// struct Id {
///     name: u32,
/// }
///
/// #[derive(Debug, IdOrdEqHash)]
/// struct Struct {
///     id: Id,
/// }
///
/// /* which will expand into:
/// impl Identifiable for Struct {
///     type Id = Id;
///
///     #[inline]
///     fn id(&self) -> &Self::Id {
///         &self.id
///     }
/// }
///
/// impl core::cmp::PartialOrd for Struct {
///     #[inline]
///     fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
///         Some(self.cmp(other))
///     }
/// }
///
/// impl core::cmp::Ord for Struct {
///     fn cmp(&self, other: &Self) -> core::cmp::Ordering {
///         self.id().cmp(other.id())
///     }
/// }
///
/// impl core::cmp::PartialEq for Struct {
///     fn eq(&self, other: &Self) -> bool {
///         self.id() == other.id()
///     }
/// }
///
/// impl core::cmp::Eq for Struct {}
///
/// impl core::hash::Hash for Struct {
///     fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
///         self.id().hash(state);
///     }
/// }*/
/// ```
///
/// Manual selection of the identifier field:
///
/// ```rust
/// use iroha_data_model_derive::IdOrdEqHash;
/// use iroha_data_model::Identifiable;
///
/// #[derive(Debug, IdOrdEqHash)]
/// struct InnerStruct {
///     #[id]
///     field: Id,
/// }
///
/// #[derive(Debug, IdOrdEqHash)]
/// struct Struct {
///     #[id(transparent)]
///     inner: InnerStruct,
/// }
///
/// #[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// struct Id {
///     name: u32,
/// }
/// ```
///
#[proc_macro_error::proc_macro_error]
#[proc_macro_derive(IdOrdEqHash, attributes(id))]
pub fn id_derive(input: TokenStream) -> TokenStream {
    id::impl_id(&parse_macro_input!(input)).into()
}

/// [`Filter`] is used for code generation of `...Filter` structs and `...EventFilter` enums, as well as
/// implementing the `Filter` trait for both of them.
/// This macro should only be attributed to `Event` enums. E.g. if the event is called `AccountEvent`,
/// then the macro will produce `AccountEventFilter` and `AccountFilter`. The latter will have `new` and
/// field getters defined, and both will have their respective `Filter` trait impls generated.
/// Due to name scoping, the macro currently properly
/// expands only from within the `iroha_data_model` crate as it relies on a few of `crate::prelude`
/// imports. This macro also depends on the naming conventions adopted so far, such as that
/// `Event` enums always have tuple variants with either some sort of `Id` or another `Event` inside
/// of them, as well as that all `Event` inner fields precede `Id` fields in the enum definition.
///
/// # Examples
///
/// ```ignore
/// use iroha_data_model_derive::{Filter, IdOrdEqHash};
/// use iroha_data_model::prelude::{HasOrigin, Identifiable};
/// use iroha_schema::IntoSchema;
/// use parity_scale_codec::{Decode, Encode};
/// use serde::{Deserialize, Serialize};
///
///
/// #[derive(Filter, Debug, Clone, PartialOrd, Ord, PartialEq, Eq, Hash, Decode, Encode, Serialize, Deserialize, IntoSchema)]
/// pub enum LayerEvent {
///     SubLayer(SubLayerEvent),
///     Created(LayerId),
/// }
///
/// #[derive(Filter, Debug, Clone, PartialOrd, Ord, PartialEq, Eq, Hash, Decode, Encode, Serialize, Deserialize, IntoSchema)]
/// pub enum SubLayerEvent {
///     Created(SubLayerId),
/// }
///
/// #[derive(Debug, Clone, PartialOrd, Ord, PartialEq, Eq, Hash, Decode, Encode, Serialize, Deserialize, IntoSchema)]
/// pub struct LayerId {
///     name: u32,
/// }
///
/// #[derive(Debug, Clone, PartialOrd, Ord, PartialEq, Eq, Hash, Decode, Encode, Serialize, Deserialize, IntoSchema)]
/// pub struct SubLayerId {
///     name: u32,
///     parent_id: LayerId,
/// }
///
/// #[derive(Debug, Clone, IdOrdEqHash)]
/// pub struct Layer {
///     id: <Self as Identifiable>::Id,
/// }
///
/// #[derive(Debug, Clone, IdOrdEqHash)]
/// pub struct SubLayer {
///     id: <Self as Identifiable>::Id,
/// }
///
/// impl HasOrigin for LayerEvent {
///     type Origin = Layer;
///
///     fn origin_id(&self) -> &<Layer as Identifiable>::Id {
///         match self {
///             Self::SubLayer(sub_layer) => &sub_layer.origin_id().parent_id,
///             Self::Created(id) => id,
///         }
///     }
/// }
///
/// impl HasOrigin for SubLayerEvent {
///     type Origin = SubLayer;
///
///     fn origin_id(&self) -> &<SubLayer as Identifiable>::Id {
///         match self {
///             Self::Created(id) => id,
///         }
///     }
/// }
/// ```
///
/// Deriving [`Filter`] for `LayerEvent` expands into:
///
/// ```
/// /*
/// #[doc = " Filter for LayerEvent entity"]
/// #[derive(Clone, PartialEq, PartialOrd, Ord, Eq, Debug, Decode, Encode, Deserialize, Serialize, IntoSchema, Hash)]
/// pub struct LayerFilter {
///     origin_filter:
///         crate::prelude::FilterOpt<crate::prelude::OriginFilter<crate::prelude::LayerEvent>>,
///     event_filter: crate::prelude::FilterOpt<LayerEventFilter>,
/// }
/// impl LayerFilter {
///     #[doc = " Construct new LayerFilter"]
///     pub const fn new(
///         origin_filter: crate::prelude::FilterOpt<
///             crate::prelude::OriginFilter<crate::prelude::LayerEvent>,
///         >,
///         event_filter: crate::prelude::FilterOpt<LayerEventFilter>,
///     ) -> Self {
///         Self {
///             origin_filter,
///             event_filter,
///         }
///     }
///     #[doc = r" Get `origin_filter`"]
///     #[inline]
///     pub const fn origin_filter(
///         &self,
///     ) -> &crate::prelude::FilterOpt<crate::prelude::OriginFilter<crate::prelude::LayerEvent>> {
///         &self.origin_filter
///     }
///     #[doc = r" Get `event_filter`"]
///     #[inline]
///     pub const fn event_filter(&self) -> &crate::prelude::FilterOpt<LayerEventFilter> {
///         &self.event_filter
///     }
/// }
/// impl crate::prelude::Filter for LayerFilter {
///     type EventType = crate::prelude::LayerEvent;
///     fn matches(&self, event: &Self::EventType) -> bool {
///         self.origin_filter.matches(event) && self.event_filter.matches(event)
///     }
/// }
/// #[doc = " Event filter for LayerEvent entity"]
/// #[allow(clippy::enum_variant_names, missing_docs)]
/// #[derive(Clone, PartialEq, PartialOrd, Ord, Eq, Debug, Decode, Encode, Deserialize, Serialize, IntoSchema, Hash)]
/// pub enum LayerEventFilter {
///     ByCreated,
///     BySubLayer(crate::prelude::FilterOpt<SubLayerFilter>),
/// }
/// impl crate::prelude::Filter for LayerEventFilter {
///     type EventType = crate::prelude::LayerEvent;
///     fn matches(&self, event: &crate::prelude::LayerEvent) -> bool {
///         match (self, event) {
///             (Self::ByCreated, crate::prelude::LayerEvent::Created(_)) => true,
///             (Self::BySubLayer(filter_opt), crate::prelude::LayerEvent::SubLayer(event)) => {
///                 filter_opt.matches(event)
///             }
///             _ => false,
///         }
///     }
/// } */
/// ```
#[proc_macro_derive(Filter)]
pub fn filter_derive(input: TokenStream) -> TokenStream {
    let event = parse_macro_input!(input as filter::EventEnum);
    filter::impl_filter(&event)
}

/// Derive `::serde::Serialize` trait for `enum` with possibility to avoid tags for selected variants
///
/// ```
/// use serde::Serialize;
/// use iroha_data_model_derive::PartiallyTaggedSerialize;
///
/// #[derive(PartiallyTaggedSerialize)]
/// enum Outer {
///     A(u64),
///     #[serde_partially_tagged(untagged)]
///     Inner(Inner),
/// }
///
/// #[derive(Serialize)]
/// enum Inner {
///     B(u32),
/// }
///
/// assert_eq!(
///     &serde_json::to_string(&Outer::Inner(Inner::B(42))).expect("Failed to serialize"), r#"{"B":42}"#
/// );
///
/// assert_eq!(
///     &serde_json::to_string(&Outer::A(42)).expect("Failed to serialize"), r#"{"A":42}"#
/// );
/// ```
#[proc_macro_error::proc_macro_error]
#[proc_macro_derive(PartiallyTaggedSerialize, attributes(serde_partially_tagged, serde))]
pub fn partially_tagged_serialize_derive(input: TokenStream) -> TokenStream {
    partially_tagged::impl_partially_tagged_serialize(&parse_macro_input!(input))
}

/// Derive `::serde::Deserialize` trait for `enum` with possibility to avoid tags for selected variants
///
/// ```
/// use serde::Deserialize;
/// use iroha_data_model_derive::PartiallyTaggedDeserialize;
///
/// #[derive(PartiallyTaggedDeserialize, PartialEq, Eq, Debug)]
/// enum Outer {
///     A(u64),
///     #[serde_partially_tagged(untagged)]
///     Inner(Inner),
/// }
///
/// #[derive(Deserialize, PartialEq, Eq, Debug)]
/// enum Inner {
///     B(u32),
/// }
///
/// assert_eq!(
///     serde_json::from_str::<Outer>(r#"{"B":42}"#).expect("Failed to deserialize"), Outer::Inner(Inner::B(42))
/// );
///
/// assert_eq!(
///     serde_json::from_str::<Outer>(r#"{"A":42}"#).expect("Failed to deserialize"), Outer::A(42)
/// );
/// ```
///
/// Deserialization of untagged variants happens in declaration order.
/// Should be used with care to avoid ambiguity.
///
/// ```
/// use serde::Deserialize;
/// use iroha_data_model_derive::PartiallyTaggedDeserialize;
///
/// #[derive(PartiallyTaggedDeserialize, PartialEq, Eq, Debug)]
/// enum Outer {
///     A(u64),
///     // Ambiguity is created here because without tag it is impossible to distinguish `Inner1` and `Inner2`.
///     // Due to deserialization order `Inner1` will be deserialized in case of ambiguity.
///     #[serde_partially_tagged(untagged)]
///     Inner1(Inner),
///     #[serde_partially_tagged(untagged)]
///     Inner2(Inner),
/// }
///
/// #[derive(Deserialize, PartialEq, Eq, Debug)]
/// enum Inner {
///     B(u32),
/// }
///
/// assert_eq!(
///     serde_json::from_str::<Outer>(r#"{"B":42}"#).expect("Failed to deserialize"), Outer::Inner1(Inner::B(42))
/// );
/// ```
#[proc_macro_error::proc_macro_error]
#[proc_macro_derive(PartiallyTaggedDeserialize, attributes(serde_partially_tagged, serde))]
pub fn partially_tagged_deserialize_derive(input: TokenStream) -> TokenStream {
    partially_tagged::impl_partially_tagged_deserialize(&parse_macro_input!(input))
}

/// Derive macro for `HasOrigin`.
///
/// Works only with enums containing single unnamed fields.
///
/// # Attributes
///
/// ## Container attributes
///
/// ### `#[has_origin(origin = Type)]`
///
/// Required attribute. Used to determine type of `Origin` in `HasOrigin` trait.
///
/// ## Field attributes
///
/// ### `#[has_origin(ident => expr)]`
///
/// This attribute is used to determine how to extract origin id from enum variant.
/// By default variant is assumed to by origin id.
///
/// # Examples
///
/// ```
/// use iroha_data_model_derive::{IdOrdEqHash, HasOrigin};
/// use iroha_data_model::prelude::{Identifiable, HasOrigin};
///
///
/// #[derive(HasOrigin, Clone, Debug)]
/// #[has_origin(origin = Layer)]
/// pub enum LayerEvent {
///     #[has_origin(sub_layer_event => &sub_layer_event.origin_id().parent_id)]
///     SubLayer(SubLayerEvent),
///     Created(LayerId),
/// }
///
/// #[derive(HasOrigin, Clone, Debug)]
/// #[has_origin(origin = SubLayer)]
/// pub enum SubLayerEvent {
///     Created(SubLayerId),
/// }
///
/// #[derive(Debug, Clone, PartialOrd, Ord, PartialEq, Eq, Hash)]
/// pub struct LayerId {
///     name: u32,
/// }
///
/// #[derive(Debug, Clone, PartialOrd, Ord, PartialEq, Eq, Hash)]
/// pub struct SubLayerId {
///     name: u32,
///     parent_id: LayerId,
/// }
///
/// #[derive(IdOrdEqHash, Debug, Clone)]
/// pub struct Layer {
///     id: LayerId,
/// }
///
/// #[derive(IdOrdEqHash, Debug, Clone)]
/// pub struct SubLayer {
///     id: SubLayerId,
/// }
///
/// let layer_id = LayerId { name: 42 };
/// let sub_layer_id = SubLayerId { name: 24, parent_id: layer_id.clone() };
/// let layer_created_event = LayerEvent::Created(layer_id.clone());
/// let sub_layer_created_event = SubLayerEvent::Created(sub_layer_id.clone());
/// let layer_sub_layer_event = LayerEvent::SubLayer(sub_layer_created_event.clone());
///
/// assert_eq!(&layer_id, layer_created_event.origin_id());
/// assert_eq!(&layer_id, layer_sub_layer_event.origin_id());
/// assert_eq!(&sub_layer_id, sub_layer_created_event.origin_id());
/// ```
#[proc_macro_error::proc_macro_error]
#[proc_macro_derive(HasOrigin, attributes(has_origin))]
pub fn has_origin_derive(input: TokenStream) -> TokenStream {
    has_origin::impl_has_origin(&parse_macro_input!(input))
}