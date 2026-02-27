use serde::Serialize;

#[derive(Serialize, Debug)]
pub struct FirstbaseDocument {
    #[serde(rename = "TradeItem")]
    pub trade_item: TradeItem,
    #[serde(rename = "CatalogueItemChildItemLink", skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<CatalogueItemChildItemLink>,
}

#[derive(Serialize, Debug)]
pub struct CatalogueItemChildItemLink {
    #[serde(rename = "Quantity")]
    pub quantity: u32,
    #[serde(rename = "CatalogueItem")]
    pub catalogue_item: CatalogueItem,
}

#[derive(Serialize, Debug)]
pub struct CatalogueItem {
    #[serde(rename = "Identifier")]
    pub identifier: String,
    #[serde(rename = "TradeItem")]
    pub trade_item: TradeItem,
    #[serde(rename = "CatalogueItemChildItemLink", skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<CatalogueItemChildItemLink>,
}

#[derive(Serialize, Debug, Default)]
pub struct TradeItem {
    #[serde(rename = "IsBrandBankPublication")]
    pub is_brand_bank_publication: bool,
    #[serde(rename = "TargetSector")]
    pub target_sector: Vec<String>,
    #[serde(rename = "ChemicalRegulationInformationModule", skip_serializing_if = "Option::is_none")]
    pub chemical_regulation_module: Option<ChemicalRegulationInformationModule>,
    #[serde(rename = "HealthcareItemInformationModule", skip_serializing_if = "Option::is_none")]
    pub healthcare_item_module: Option<HealthcareItemInformationModule>,
    #[serde(rename = "MedicalDeviceTradeItemModule")]
    pub medical_device_module: MedicalDeviceTradeItemModule,
    #[serde(rename = "ReferencedFileDetailInformationModule", skip_serializing_if = "Option::is_none")]
    pub referenced_file_module: Option<ReferencedFileDetailInformationModule>,
    #[serde(rename = "RegulatedTradeItemModule", skip_serializing_if = "Option::is_none")]
    pub regulated_trade_item_module: Option<RegulatedTradeItemModule>,
    #[serde(rename = "SalesInformationModule", skip_serializing_if = "Option::is_none")]
    pub sales_module: Option<SalesInformationModule>,
    #[serde(rename = "TradeItemDescriptionModule", skip_serializing_if = "Option::is_none")]
    pub description_module: Option<TradeItemDescriptionModule>,
    #[serde(rename = "IsTradeItemABaseUnit")]
    pub is_base_unit: bool,
    #[serde(rename = "IsTradeItemADespatchUnit")]
    pub is_despatch_unit: bool,
    #[serde(rename = "IsTradeItemAnOrderableUnit")]
    pub is_orderable_unit: bool,
    #[serde(rename = "TradeItemUnitDescriptorCode")]
    pub unit_descriptor: CodeValue,
    #[serde(rename = "TradeItemTradeChannelCode", skip_serializing_if = "Vec::is_empty")]
    pub trade_channel_code: Vec<CodeValue>,
    #[serde(rename = "InformationProviderOfTradeItem")]
    pub information_provider: InformationProvider,
    #[serde(rename = "GdsnTradeItemClassification")]
    pub classification: GdsnClassification,
    #[serde(rename = "NextLowerLevelTradeItemInformation", skip_serializing_if = "Option::is_none")]
    pub next_lower_level: Option<NextLowerLevel>,
    #[serde(rename = "TargetMarket")]
    pub target_market: TargetMarketObj,
    #[serde(rename = "TradeItemContactInformation", skip_serializing_if = "Vec::is_empty")]
    pub contact_information: Vec<TradeItemContactInformation>,
    #[serde(rename = "TradeItemSynchronisationDates")]
    pub synchronisation_dates: TradeItemSynchronisationDates,
    #[serde(rename = "GlobalModelInformation")]
    pub global_model_info: Vec<GlobalModelInformation>,
    #[serde(rename = "Gtin")]
    pub gtin: String,
    #[serde(rename = "AdditionalTradeItemIdentification", skip_serializing_if = "Vec::is_empty")]
    pub additional_identification: Vec<AdditionalTradeItemIdentification>,
}

#[derive(Serialize, Debug, Default, Clone)]
pub struct CodeValue {
    #[serde(rename = "Value")]
    pub value: String,
}

#[derive(Serialize, Debug, Default)]
pub struct InformationProvider {
    #[serde(rename = "Gln")]
    pub gln: String,
    #[serde(rename = "PartyName")]
    pub party_name: String,
}

#[derive(Serialize, Debug, Default)]
pub struct GdsnClassification {
    #[serde(rename = "GpcSegmentCode")]
    pub segment_code: String,
    #[serde(rename = "GpcClassCode")]
    pub class_code: String,
    #[serde(rename = "GpcFamilyCode")]
    pub family_code: String,
    #[serde(rename = "GpcCategoryCode")]
    pub category_code: String,
    #[serde(rename = "GpcCategoryName")]
    pub category_name: String,
    #[serde(rename = "AdditionalTradeItemClassification", skip_serializing_if = "Vec::is_empty")]
    pub additional_classifications: Vec<AdditionalClassification>,
}

#[derive(Serialize, Debug)]
pub struct AdditionalClassification {
    #[serde(rename = "AdditionalTradeItemClassificationSystemCode")]
    pub system_code: CodeValue,
    #[serde(rename = "AdditionalTradeItemClassificationValue")]
    pub values: Vec<AdditionalClassificationValue>,
}

#[derive(Serialize, Debug)]
pub struct AdditionalClassificationValue {
    #[serde(rename = "AdditionalTradeItemClassificationCodeValue")]
    pub code_value: String,
}

#[derive(Serialize, Debug)]
pub struct NextLowerLevel {
    #[serde(rename = "QuantityOfChildren")]
    pub quantity_of_children: u32,
    #[serde(rename = "TotalQuantityOfNextLowerLevelTradeItem")]
    pub total_quantity: u32,
    #[serde(rename = "ChildTradeItem")]
    pub child_items: Vec<ChildTradeItem>,
}

#[derive(Serialize, Debug)]
pub struct ChildTradeItem {
    #[serde(rename = "QuantityOfNextLowerLevelTradeItem")]
    pub quantity: u32,
    #[serde(rename = "Gtin")]
    pub gtin: String,
}

#[derive(Serialize, Debug, Default)]
pub struct TargetMarketObj {
    #[serde(rename = "TargetMarketCountryCode")]
    pub country_code: CodeValue,
}

#[derive(Serialize, Debug, Default)]
pub struct TradeItemSynchronisationDates {
    #[serde(rename = "LastChangeDateTime")]
    pub last_change: String,
    #[serde(rename = "EffectiveDateTime")]
    pub effective: String,
    #[serde(rename = "PublicationDateTime")]
    pub publication: String,
}

#[derive(Serialize, Debug)]
pub struct GlobalModelInformation {
    #[serde(rename = "GlobalModelNumber")]
    pub number: String,
    #[serde(rename = "GlobalModelDescription", skip_serializing_if = "Vec::is_empty")]
    pub descriptions: Vec<LangValue>,
}

#[derive(Serialize, Debug, Clone)]
pub struct LangValue {
    #[serde(rename = "LanguageCode")]
    pub language_code: String,
    #[serde(rename = "Value")]
    pub value: String,
}

#[derive(Serialize, Debug)]
pub struct AdditionalTradeItemIdentification {
    #[serde(rename = "AdditionalTradeItemIdentificationTypeCode")]
    pub type_code: String,
    #[serde(rename = "Value")]
    pub value: String,
}

// --- Medical Device Module ---
#[derive(Serialize, Debug, Default)]
pub struct MedicalDeviceTradeItemModule {
    #[serde(rename = "MedicalDeviceInformation")]
    pub info: MedicalDeviceInformation,
}

#[derive(Serialize, Debug, Default)]
pub struct MedicalDeviceInformation {
    #[serde(rename = "IsTradeItemImplantable", skip_serializing_if = "Option::is_none")]
    pub is_implantable: Option<String>,
    #[serde(rename = "UdidDeviceCount", skip_serializing_if = "Option::is_none")]
    pub device_count: Option<u32>,
    #[serde(rename = "DirectPartMarkingIdentifier", skip_serializing_if = "Vec::is_empty")]
    pub direct_marking: Vec<DirectPartMarking>,
    #[serde(rename = "HasDeviceMeasuringFunction", skip_serializing_if = "Option::is_none")]
    pub measuring_function: Option<bool>,
    #[serde(rename = "IsActiveDevice", skip_serializing_if = "Option::is_none")]
    pub is_active: Option<bool>,
    #[serde(rename = "IsDeviceIntendedToAdministerOrRemoveMedicinalProduct", skip_serializing_if = "Option::is_none")]
    pub administer_medicine: Option<bool>,
    #[serde(rename = "IsDeviceMedicinalProduct", skip_serializing_if = "Option::is_none")]
    pub is_medicinal_product: Option<bool>,
    #[serde(rename = "IsReprocessedSingleUseDevice", skip_serializing_if = "Option::is_none")]
    pub is_reprocessed: Option<bool>,
    #[serde(rename = "IsReusableSurgicalInstrument", skip_serializing_if = "Option::is_none")]
    pub is_reusable_surgical: Option<bool>,
    #[serde(rename = "UDIProductionIdentifierTypeCode", skip_serializing_if = "Vec::is_empty")]
    pub production_identifier_types: Vec<CodeValue>,
    #[serde(rename = "AnnexXVIIntendedPurposeTypeCode", skip_serializing_if = "Vec::is_empty")]
    pub annex_xvi_types: Vec<CodeValue>,
    #[serde(rename = "MultiComponentDeviceTypeCode", skip_serializing_if = "Option::is_none")]
    pub multi_component_type: Option<CodeValue>,
    #[serde(rename = "EUMedicalDeviceStatusCode")]
    pub eu_status: CodeValue,
    #[serde(rename = "HealthcareTradeItemReusabilityInformation", skip_serializing_if = "Option::is_none")]
    pub reusability: Option<ReusabilityInformation>,
    #[serde(rename = "TradeItemSterilityInformation", skip_serializing_if = "Option::is_none")]
    pub sterility: Option<SterilityInformation>,
}

#[derive(Serialize, Debug)]
pub struct DirectPartMarking {
    #[serde(rename = "IdentificationSchemeAgencyCode")]
    pub agency_code: String,
    #[serde(rename = "Value")]
    pub value: String,
}

#[derive(Serialize, Debug)]
pub struct ReusabilityInformation {
    #[serde(rename = "ManufacturerDeclaredReusabilityTypeCode")]
    pub reusability_type: CodeValue,
    #[serde(rename = "MaximumCyclesReusable", skip_serializing_if = "Option::is_none")]
    pub max_cycles: Option<u32>,
}

#[derive(Serialize, Debug)]
pub struct SterilityInformation {
    #[serde(rename = "InitialManufacturerSterilisationCode")]
    pub manufacturer_sterilisation: Vec<CodeValue>,
    #[serde(rename = "InitialSterilisationPriorToUseCode", skip_serializing_if = "Vec::is_empty")]
    pub prior_to_use: Vec<CodeValue>,
}

// --- Healthcare Item Information Module ---
#[derive(Serialize, Debug)]
pub struct HealthcareItemInformationModule {
    #[serde(rename = "HealthcareItemInformation")]
    pub info: HealthcareItemInformation,
}

#[derive(Serialize, Debug)]
pub struct HealthcareItemInformation {
    #[serde(rename = "DoesTradeItemContainHumanBloodDerivative", skip_serializing_if = "Option::is_none")]
    pub human_blood_derivative: Option<String>,
    #[serde(rename = "DoesTradeItemContainLatex", skip_serializing_if = "Option::is_none")]
    pub contains_latex: Option<String>,
    #[serde(rename = "DoesTradeItemContainHumanTissue", skip_serializing_if = "Option::is_none")]
    pub human_tissue: Option<String>,
    #[serde(rename = "DoesTradeItemContainAnimalTissue", skip_serializing_if = "Option::is_none")]
    pub animal_tissue: Option<serde_json::Value>,
    #[serde(rename = "ClinicalStorageHandlingInformation", skip_serializing_if = "Vec::is_empty")]
    pub storage_handling: Vec<ClinicalStorageHandling>,
    #[serde(rename = "ClinicalSize", skip_serializing_if = "Vec::is_empty")]
    pub clinical_sizes: Vec<ClinicalSizeOutput>,
    #[serde(rename = "ClinicalWarning", skip_serializing_if = "Vec::is_empty")]
    pub clinical_warnings: Vec<ClinicalWarningOutput>,
}

#[derive(Serialize, Debug)]
pub struct ClinicalStorageHandling {
    #[serde(rename = "ClinicalStorageHandlingTypeCode")]
    pub type_code: CodeValue,
    #[serde(rename = "ClinicalStorageHandlingDescription", skip_serializing_if = "Vec::is_empty")]
    pub descriptions: Vec<LangValue>,
}

#[derive(Serialize, Debug)]
pub struct ClinicalSizeOutput {
    #[serde(rename = "ClinicalSizeTypeCode")]
    pub type_code: CodeValue,
    #[serde(rename = "ClinicalSizeValue", skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<MeasurementValue>,
    #[serde(rename = "ClinicalSizeValueMaximum", skip_serializing_if = "Vec::is_empty")]
    pub maximums: Vec<MeasurementValue>,
    #[serde(rename = "ClinicalSizeMeasurementPrecisionCode")]
    pub precision: CodeValue,
    #[serde(rename = "ClinicalSizeValueText", skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct MeasurementValue {
    #[serde(rename = "MeasurementUnitCode")]
    pub unit_code: String,
    #[serde(rename = "Value")]
    pub value: f64,
}

#[derive(Serialize, Debug)]
pub struct ClinicalWarningOutput {
    #[serde(rename = "ClinicalWarningAgencyCode")]
    pub agency_code: CodeValue,
    #[serde(rename = "ClinicalWarningCode")]
    pub warning_code: String,
    #[serde(rename = "WarningsOrContraIndicationDescription", skip_serializing_if = "Vec::is_empty")]
    pub descriptions: Vec<LangValue>,
}

// --- Chemical Regulation Module ---
#[derive(Serialize, Debug)]
pub struct ChemicalRegulationInformationModule {
    #[serde(rename = "ChemicalRegulationInformation")]
    pub infos: Vec<ChemicalRegulationInformation>,
}

#[derive(Serialize, Debug)]
pub struct ChemicalRegulationInformation {
    #[serde(rename = "ChemicalRegulationAgency")]
    pub agency: String,
    #[serde(rename = "ChemicalRegulation")]
    pub regulations: Vec<ChemicalRegulation>,
}

#[derive(Serialize, Debug)]
pub struct ChemicalRegulation {
    #[serde(rename = "ChemicalRegulationName")]
    pub regulation_name: String,
    #[serde(rename = "RegulatedChemical")]
    pub chemicals: Vec<RegulatedChemical>,
}

#[derive(Serialize, Debug)]
pub struct RegulatedChemical {
    #[serde(rename = "RegulatedChemicalIdentifierCodeReference", skip_serializing_if = "Option::is_none")]
    pub identifier_ref: Option<ChemicalIdentifierRef>,
    #[serde(rename = "RegulatedChemicalName", skip_serializing_if = "Option::is_none")]
    pub chemical_name: Option<String>,
    #[serde(rename = "RegulatedChemicalDescription", skip_serializing_if = "Vec::is_empty")]
    pub descriptions: Vec<LangValue>,
    #[serde(rename = "CarcinogenicMutagenicReprotoxicTypeCode", skip_serializing_if = "Option::is_none")]
    pub cmr_type: Option<CodeValue>,
    #[serde(rename = "RegulatedChemicalTypeCode")]
    pub chemical_type: CodeValue,
}

#[derive(Serialize, Debug)]
pub struct ChemicalIdentifierRef {
    #[serde(rename = "CodeListAgencyName")]
    pub agency_name: String,
    #[serde(rename = "Value")]
    pub value: String,
}

// --- Referenced File Module ---
#[derive(Serialize, Debug)]
pub struct ReferencedFileDetailInformationModule {
    #[serde(rename = "ReferencedFileHeader")]
    pub headers: Vec<ReferencedFileHeader>,
}

#[derive(Serialize, Debug)]
pub struct ReferencedFileHeader {
    #[serde(rename = "MediaSourceGln", skip_serializing_if = "Option::is_none")]
    pub media_source_gln: Option<String>,
    #[serde(rename = "MimeType", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(rename = "ReferencedFileTypeCode")]
    pub file_type: CodeValue,
    #[serde(rename = "FileFormatName", skip_serializing_if = "Option::is_none")]
    pub format_name: Option<String>,
    #[serde(rename = "FileName", skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(rename = "UniformResourceIdentifier")]
    pub uri: String,
    #[serde(rename = "IsPrimaryFile")]
    pub is_primary: String,
}

// --- Regulated Trade Item Module ---
#[derive(Serialize, Debug)]
pub struct RegulatedTradeItemModule {
    #[serde(rename = "RegulatoryInformation")]
    pub info: Vec<RegulatoryInformation>,
}

#[derive(Serialize, Debug)]
pub struct RegulatoryInformation {
    #[serde(rename = "RegulatoryAct")]
    pub act: String,
    #[serde(rename = "RegulatoryAgency")]
    pub agency: String,
}

// --- Sales Information Module ---
#[derive(Serialize, Debug)]
pub struct SalesInformationModule {
    #[serde(rename = "SalesInformation")]
    pub sales: SalesInformation,
}

#[derive(Serialize, Debug)]
pub struct SalesInformation {
    #[serde(rename = "TargetMarketSalesConditions")]
    pub conditions: Vec<TargetMarketSalesCondition>,
}

#[derive(Serialize, Debug)]
pub struct TargetMarketSalesCondition {
    #[serde(rename = "TargetMarketConsumerSalesConditionCode")]
    pub condition_code: CodeValue,
    #[serde(rename = "SalesConditionTargetMarketCountry")]
    pub countries: Vec<SalesConditionCountry>,
}

#[derive(Serialize, Debug)]
pub struct SalesConditionCountry {
    #[serde(rename = "CountryCode")]
    pub country_code: CodeValue,
    #[serde(rename = "EndAvailabilityDateTime", skip_serializing_if = "Option::is_none")]
    pub end_datetime: Option<String>,
    #[serde(rename = "StartAvailabilityDateTime")]
    pub start_datetime: String,
}

// --- Trade Item Description Module ---
#[derive(Serialize, Debug)]
pub struct TradeItemDescriptionModule {
    #[serde(rename = "TradeItemDescriptionInformation")]
    pub info: TradeItemDescriptionInformation,
}

#[derive(Serialize, Debug)]
pub struct TradeItemDescriptionInformation {
    #[serde(rename = "AdditionalTradeItemDescription", skip_serializing_if = "Vec::is_empty")]
    pub additional_descriptions: Vec<LangValue>,
    #[serde(rename = "TradeItemDescription", skip_serializing_if = "Vec::is_empty")]
    pub descriptions: Vec<LangValue>,
}

// --- Contact Information ---
#[derive(Serialize, Debug)]
pub struct TradeItemContactInformation {
    #[serde(rename = "ContactTypeCode")]
    pub contact_type: CodeValue,
    #[serde(rename = "AdditionalPartyIdentification", skip_serializing_if = "Vec::is_empty")]
    pub party_identification: Vec<AdditionalPartyIdentification>,
    #[serde(rename = "ContactName", skip_serializing_if = "Option::is_none")]
    pub contact_name: Option<String>,
    #[serde(rename = "StructuredAddress", skip_serializing_if = "Vec::is_empty")]
    pub addresses: Vec<StructuredAddress>,
    #[serde(rename = "TargetMarketCommunicationChannel", skip_serializing_if = "Vec::is_empty")]
    pub communication_channels: Vec<TargetMarketCommunicationChannel>,
}

#[derive(Serialize, Debug)]
pub struct AdditionalPartyIdentification {
    #[serde(rename = "AdditionalPartyIdentificationTypeCode")]
    pub type_code: String,
    #[serde(rename = "Value")]
    pub value: String,
}

#[derive(Serialize, Debug)]
pub struct StructuredAddress {
    #[serde(rename = "City")]
    pub city: String,
    #[serde(rename = "CountryCode")]
    pub country_code: CodeValue,
    #[serde(rename = "PostalCode")]
    pub postal_code: String,
    #[serde(rename = "StreetAddress")]
    pub street: String,
    #[serde(rename = "StreetNumber", skip_serializing_if = "Option::is_none")]
    pub street_number: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct TargetMarketCommunicationChannel {
    #[serde(rename = "CommunicationChannel")]
    pub channels: Vec<CommunicationChannel>,
}

#[derive(Serialize, Debug)]
pub struct CommunicationChannel {
    #[serde(rename = "CommunicationChannelCode")]
    pub channel_code: CodeValue,
    #[serde(rename = "CommunicationValue")]
    pub value: String,
}
