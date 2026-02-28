use anyhow::{Context, Result};

// ---- Domain structs (populated manually from DOM) ----

#[derive(Debug, Default)]
pub struct PullResponse {
    pub correlation_id: Option<String>,
    pub creation_date_time: Option<String>,
    pub device: Device,
}

#[derive(Debug, Default)]
pub struct Device {
    pub device_type: Option<String>,
    pub mdr_basic_udi: Option<MdrBasicUdi>,
    pub mdr_udidi_data: Option<MdrUdidiData>,
}

#[derive(Debug, Default)]
pub struct MdrBasicUdi {
    pub risk_class: Option<String>,
    pub model_name: Option<ModelName>,
    pub identifier: Option<DiIdentifier>,
    pub animal_tissues_cells: Option<bool>,
    pub ar_actor_code: Option<String>,
    pub human_tissues_cells: Option<bool>,
    pub mf_actor_code: Option<String>,
    pub human_product_check: Option<bool>,
    pub medicinal_product_check: Option<bool>,
    pub device_kind: Option<String>,
    pub active: Option<bool>,
    pub administering_medicine: Option<bool>,
    pub implantable: Option<bool>,
    pub measuring_function: Option<bool>,
    pub reusable: Option<bool>,
}

#[derive(Debug, Default)]
pub struct ModelName {
    pub model: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Default, Clone)]
#[allow(dead_code)]
pub struct DiIdentifier {
    pub di_code: Option<String>,
    pub issuing_entity_code: Option<String>,
}

#[derive(Debug, Default)]
#[allow(dead_code)]
pub struct MdrUdidiData {
    pub identifier: Option<DiIdentifier>,
    pub status: Option<String>,
    pub additional_description: Option<Vec<LanguageSpecificName>>,
    pub basic_udi_identifier: Option<DiIdentifier>,
    pub mdn_codes: Option<String>,
    pub production_identifier: Option<String>,
    pub reference_number: Option<String>,
    pub sterile: Option<bool>,
    pub sterilization: Option<bool>,
    pub trade_names: Option<Vec<LanguageSpecificName>>,
    pub website: Option<String>,
    pub storage_handling_conditions: Vec<StorageCondition>,
    pub packages: Vec<Package>,
    pub critical_warnings: Vec<Warning>,
    pub number_of_reuses: Option<u32>,
    pub market_infos: Vec<MarketInfo>,
    pub base_quantity: Option<u32>,
    pub product_designer_actor: Option<ProductDesignerActor>,
    pub annex_xvi_types: Vec<String>,
    pub latex: Option<bool>,
    pub reprocessed: Option<bool>,
    pub substances: Vec<Substance>,
    pub clinical_sizes: Vec<ClinicalSize>,
}

#[derive(Debug, Default, Clone)]
pub struct LanguageSpecificName {
    pub language: Option<String>,
    pub text_value: Option<String>,
}

#[derive(Debug, Default)]
pub struct StorageCondition {
    pub comments: Vec<LanguageSpecificName>,
    pub value: Option<String>,
}

#[derive(Debug, Default)]
pub struct Package {
    pub identifier: Option<DiIdentifier>,
    pub child: Option<DiIdentifier>,
    pub number_of_items: Option<u32>,
}

#[derive(Debug, Default)]
pub struct Warning {
    pub comments: Vec<LanguageSpecificName>,
    pub warning_value: Option<String>,
}

#[derive(Debug, Default)]
pub struct MarketInfo {
    pub country: Option<String>,
    pub original_placed: Option<bool>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

#[derive(Debug, Default)]
pub struct ProductDesignerActor {
    pub organisation: Option<ProductDesignerOrganisation>,
}

#[derive(Debug, Default)]
pub struct ProductDesignerOrganisation {
    pub address: Option<Address>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub org_name: Option<String>,
}

#[derive(Debug, Default)]
pub struct Address {
    pub city: Option<String>,
    pub country: Option<String>,
    pub post_code: Option<String>,
    pub street: Option<String>,
    pub street_num: Option<String>,
}

#[derive(Debug, Default)]
pub struct Substance {
    pub substance_type: Option<String>,  // from xsi:type: CMRSubstanceType, EndocrineSubstanceType, etc.
    pub names: Vec<LanguageSpecificName>,
    pub inn: Option<String>,
    pub sub_type: Option<String>,  // from <type> element
}

#[derive(Debug, Default)]
pub struct ClinicalSize {
    pub size_type: Option<String>,   // from xsi:type: RangeClinicalSizeType, etc.
    pub clinical_size_type: Option<String>,
    pub maximum: Option<String>,
    pub minimum: Option<String>,
    pub value: Option<String>,
    pub text: Option<String>,
    pub value_unit: Option<String>,
}

// ---- Parsing with roxmltree ----

fn local_name<'a>(node: &'a roxmltree::Node) -> &'a str {
    node.tag_name().name()
}

fn child_text<'a>(parent: &'a roxmltree::Node, name: &str) -> Option<String> {
    parent.children()
        .find(|c| c.is_element() && local_name(c) == name)
        .and_then(|c| c.text().map(|t| t.to_string()))
}

fn child_bool(parent: &roxmltree::Node, name: &str) -> Option<bool> {
    child_text(parent, name).map(|s| s.to_lowercase() == "true")
}

fn child_u32(parent: &roxmltree::Node, name: &str) -> Option<u32> {
    child_text(parent, name).and_then(|s| s.parse().ok())
}

fn child_element<'a, 'b>(parent: &'a roxmltree::Node<'a, 'b>, name: &str) -> Option<roxmltree::Node<'a, 'b>> {
    parent.children().find(|c| c.is_element() && local_name(c) == name)
}

fn parse_di_identifier(node: &roxmltree::Node) -> DiIdentifier {
    DiIdentifier {
        di_code: child_text(node, "DICode"),
        issuing_entity_code: child_text(node, "issuingEntityCode"),
    }
}

fn parse_lang_names(parent: &roxmltree::Node) -> Vec<LanguageSpecificName> {
    parent.children()
        .filter(|c| c.is_element() && local_name(c) == "name")
        .map(|n| LanguageSpecificName {
            language: child_text(&n, "language"),
            text_value: child_text(&n, "textValue"),
        })
        .collect()
}

fn xsi_type_local(node: &roxmltree::Node) -> Option<String> {
    // Get xsi:type attribute value and strip namespace prefix
    let xsi_ns = "http://www.w3.org/2001/XMLSchema-instance";
    node.attribute((xsi_ns, "type"))
        .map(|v| {
            if let Some(pos) = v.find(':') {
                v[pos+1..].to_string()
            } else {
                v.to_string()
            }
        })
}

fn parse_basic_udi(node: &roxmltree::Node) -> MdrBasicUdi {
    let model_name_node = child_element(node, "modelName");
    let model_name = model_name_node.map(|mn| ModelName {
        model: child_text(&mn, "model"),
        name: child_text(&mn, "name"),
    });

    let identifier = child_element(node, "identifier").map(|n| parse_di_identifier(&n));

    MdrBasicUdi {
        risk_class: child_text(node, "riskClass"),
        model_name,
        identifier,
        animal_tissues_cells: child_bool(node, "animalTissuesCells"),
        ar_actor_code: child_text(node, "ARActorCode"),
        human_tissues_cells: child_bool(node, "humanTissuesCells"),
        mf_actor_code: child_text(node, "MFActorCode"),
        human_product_check: child_bool(node, "humanProductCheck"),
        medicinal_product_check: child_bool(node, "medicinalProductCheck"),
        device_kind: child_text(node, "type"),
        active: child_bool(node, "active"),
        administering_medicine: child_bool(node, "administeringMedicine"),
        implantable: child_bool(node, "implantable"),
        measuring_function: child_bool(node, "measuringFunction"),
        reusable: child_bool(node, "reusable"),
    }
}

fn parse_udidi_data(node: &roxmltree::Node) -> MdrUdidiData {
    let identifier = child_element(node, "identifier").map(|n| parse_di_identifier(&n));
    let status = child_element(node, "status")
        .and_then(|s| child_text(&s, "code"));
    let additional_description = child_element(node, "additionalDescription")
        .map(|n| parse_lang_names(&n));
    let basic_udi_identifier = child_element(node, "basicUDIIdentifier")
        .map(|n| parse_di_identifier(&n));
    let trade_names = child_element(node, "tradeNames")
        .map(|n| parse_lang_names(&n));

    // Storage handling conditions
    let storage = child_element(node, "storageHandlingConditions")
        .map(|shc| {
            shc.children()
                .filter(|c| c.is_element() && local_name(c) == "condition")
                .map(|cond| {
                    let comments_node = child_element(&cond, "comments");
                    let comments = comments_node.map(|c| parse_lang_names(&c)).unwrap_or_default();
                    StorageCondition {
                        comments,
                        value: child_text(&cond, "storageHandlingConditionValue"),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // Packages
    let packages = child_element(node, "packages")
        .map(|pkgs| {
            pkgs.children()
                .filter(|c| c.is_element() && local_name(c) == "package")
                .map(|pkg| Package {
                    identifier: child_element(&pkg, "identifier").map(|n| parse_di_identifier(&n)),
                    child: child_element(&pkg, "child").map(|n| parse_di_identifier(&n)),
                    number_of_items: child_u32(&pkg, "numberOfItems"),
                })
                .collect()
        })
        .unwrap_or_default();

    // Critical warnings
    let warnings = child_element(node, "criticalWarnings")
        .map(|cw| {
            cw.children()
                .filter(|c| c.is_element() && local_name(c) == "warning")
                .map(|w| {
                    let comments_node = child_element(&w, "comments");
                    let comments = comments_node.map(|c| parse_lang_names(&c)).unwrap_or_default();
                    Warning {
                        comments,
                        warning_value: child_text(&w, "warningValue"),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // Market infos
    let market_infos = child_element(node, "marketInfos")
        .map(|mi| {
            mi.children()
                .filter(|c| c.is_element() && local_name(c) == "marketInfo")
                .map(|info| MarketInfo {
                    country: child_text(&info, "country"),
                    original_placed: child_bool(&info, "originalPlacedOnTheMarket"),
                    start_date: child_text(&info, "startDate"),
                    end_date: child_text(&info, "endDate"),
                })
                .collect()
        })
        .unwrap_or_default();

    // Product designer
    let product_designer = child_element(node, "productDesignerActor").map(|pda| {
        let org = child_element(&pda, "productDesignerOrganisation").map(|org_node| {
            let address = child_element(&org_node, "geographicAddress").map(|addr| Address {
                city: child_text(&addr, "city"),
                country: child_text(&addr, "country"),
                post_code: child_text(&addr, "postCode"),
                street: child_text(&addr, "street"),
                street_num: child_text(&addr, "streetNum"),
            });

            let (email, phone) = if let Some(cd) = child_element(&org_node, "contactsDetails") {
                if let Some(detail) = child_element(&cd, "contactDetail") {
                    (child_text(&detail, "eMail"), child_text(&detail, "phone"))
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };

            let org_name = child_element(&org_node, "organizationName")
                .and_then(|n| child_text(&n, "textValue"));

            ProductDesignerOrganisation {
                address,
                email,
                phone,
                org_name,
            }
        });

        ProductDesignerActor { organisation: org }
    });

    // Annex XVI types
    let annex_xvi = child_element(node, "annexXVINonMedicalDeviceTypes")
        .map(|ax| {
            ax.children()
                .filter(|c| c.is_element() && local_name(c) == "nmdType")
                .filter_map(|c| c.text().map(|t| t.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // Substances
    let substances = child_element(node, "substances")
        .map(|subs| {
            subs.children()
                .filter(|c| c.is_element() && local_name(c) == "substance")
                .map(|s| {
                    let xsi = xsi_type_local(&s);
                    let names_node = child_element(&s, "names");
                    let names = names_node.map(|n| parse_lang_names(&n)).unwrap_or_default();

                    Substance {
                        substance_type: xsi,
                        names,
                        inn: child_text(&s, "INN"),
                        sub_type: child_text(&s, "type"),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // Clinical sizes
    let clinical_sizes = child_element(node, "clinicalSizes")
        .map(|cs| {
            cs.children()
                .filter(|c| c.is_element() && local_name(c) == "clinicalSize")
                .map(|s| ClinicalSize {
                    size_type: xsi_type_local(&s),
                    clinical_size_type: child_text(&s, "clinicalSizeType"),
                    maximum: child_text(&s, "maximum"),
                    minimum: child_text(&s, "minimum"),
                    value: child_text(&s, "value"),
                    text: child_text(&s, "text"),
                    value_unit: child_text(&s, "valueUnit"),
                })
                .collect()
        })
        .unwrap_or_default();

    MdrUdidiData {
        identifier,
        status,
        additional_description,
        basic_udi_identifier,
        mdn_codes: child_text(node, "MDNCodes"),
        production_identifier: child_text(node, "productionIdentifier"),
        reference_number: child_text(node, "referenceNumber"),
        sterile: child_bool(node, "sterile"),
        sterilization: child_bool(node, "sterilization"),
        trade_names,
        website: child_text(node, "website"),
        storage_handling_conditions: storage,
        packages,
        critical_warnings: warnings,
        number_of_reuses: child_u32(node, "numberOfReuses"),
        market_infos,
        base_quantity: child_u32(node, "baseQuantity"),
        product_designer_actor: product_designer,
        annex_xvi_types: annex_xvi,
        latex: child_bool(node, "latex"),
        reprocessed: child_bool(node, "reprocessed"),
        substances,
        clinical_sizes,
    }
}

/// Parse EUDAMED PullResponse XML into typed structs
pub fn parse_pull_response(xml_content: &str) -> Result<PullResponse> {
    let doc = roxmltree::Document::parse(xml_content)
        .context("Failed to parse XML")?;

    let root = doc.root_element();
    let mut response = PullResponse::default();

    response.correlation_id = child_text(&root, "correlationID");
    response.creation_date_time = child_text(&root, "creationDateTime");

    // Find payload
    let payload = child_element(&root, "payload")
        .context("Missing <payload> element")?;

    // Find Device
    let device_node = child_element(&payload, "Device")
        .context("Missing <Device> element in payload")?;

    response.device.device_type = xsi_type_local(&device_node);

    // Parse MDRBasicUDI
    if let Some(basic) = child_element(&device_node, "MDRBasicUDI") {
        response.device.mdr_basic_udi = Some(parse_basic_udi(&basic));
    }

    // Parse MDRUDIDIData
    if let Some(udidi) = child_element(&device_node, "MDRUDIDIData") {
        response.device.mdr_udidi_data = Some(parse_udidi_data(&udidi));
    }

    Ok(response)
}
