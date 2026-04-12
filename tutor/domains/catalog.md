# Product Catalog

Exercises: entity types, value types, reference schemes, binary fact types,
uniqueness constraints, mandatory constraints, value constraints, subtypes,
objectification, CRUDL, field ordering.

## Entity Types

Product(.SKU) is an entity type.
Category(.Category Code) is an entity type.
Brand(.Brand Name) is an entity type.
Review(.id) is an entity type.
ProductVariant(.id) is an entity type where Product has Size and Color.

## Subtypes

DigitalProduct is a subtype of Product.
PhysicalProduct is a subtype of Product.

## Value Types

SKU is a value type.
Category Code is a value type.
Brand Name is a value type.
Product Name is a value type.
Description is a value type.
Price is a value type.
Currency is a value type.
  The possible values of Currency are 'USD', 'EUR', 'GBP', 'JPY', 'CAD'.
Weight is a value type.
Size is a value type.
  The possible values of Size are 'XS', 'S', 'M', 'L', 'XL', 'XXL'.
Color is a value type.
Rating is a value type.
Review Text is a value type.
Download URL is a value type.
Dimensions is a value type.
Stock Count is a value type.
Is Active is a value type.
Tag is a value type.

## Readings

### Product

Product has Product Name.
  Each Product has exactly one Product Name.

Product has Description.
  Each Product has at most one Description.

Product has Price.
  Each Product has exactly one Price.

Product has Currency.
  Each Product has exactly one Currency.

Product belongs to Category.
  Each Product belongs to at most one Category.
  It is possible that some Category has more than one Product.

Product is made by Brand.
  Each Product is made by at most one Brand.
  It is possible that some Brand makes more than one Product.

Product is active.

Product has Tag.
  It is possible that some Product has more than one Tag.
  In each population of Product has Tag, each Product, Tag combination occurs at most once.

### Category

Category has Category Name.
  Each Category has exactly one Category Name.

Category has parent Category.
  Each Category has at most one parent Category.

### Brand

Brand has Description.
  Each Brand has at most one Description.

Brand has Logo URL.
  Each Brand has at most one Logo URL.

### Review

Review is for Product.
  Each Review is for exactly one Product.

Review has Rating.
  Each Review has exactly one Rating.

Review has Review Text.
  Each Review has at most one Review Text.

Review is by User.
  Each Review is by exactly one User.

### Physical Product

PhysicalProduct has Weight.
  Each PhysicalProduct has at most one Weight.

PhysicalProduct has Dimensions.
  Each PhysicalProduct has at most one Dimensions.

PhysicalProduct has Stock Count.
  Each PhysicalProduct has at most one Stock Count.

### Digital Product

DigitalProduct has Download URL.
  Each DigitalProduct has exactly one Download URL.

### Product Variant (objectification of "Product has Size and Color")

This association with Product, Size, Color provides the preferred identification scheme for ProductVariant.

ProductVariant has Price.
  Each ProductVariant has at most one Price.

ProductVariant has Stock Count.
  Each ProductVariant has at most one Stock Count.

## Constraints

Each Category has at most one parent Category.
No Category has the same Category as parent. (irreflexive)
If some Category has parent some Category1 and that Category1 has parent some Category2 then it is not the case that Category2 has parent that Category. (acyclic)

It is obligatory that each Product has exactly one Price.
It is forbidden that a Review has Rating less than 1.
It is forbidden that a Review has Rating greater than 5.

For each Product, exactly one of the following holds:
  that Product is a DigitalProduct;
  that Product is a PhysicalProduct.

## Instance Facts

Category 'electronics' has Category Name 'Electronics'.
Category 'clothing' has Category Name 'Clothing'.
Category 'books' has Category Name 'Books'.
Category 'phones' has Category Name 'Phones'.
Category 'phones' has parent Category 'electronics'.
Category 'laptops' has Category Name 'Laptops'.
Category 'laptops' has parent Category 'electronics'.

Brand 'acme' has Brand Name 'Acme Corp'.
Brand 'globex' has Brand Name 'Globex Industries'.

Domain 'catalog' has Visibility 'public'.
