terraform {
  required_version = ">= 1.7"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
  }

  backend "s3" {
    bucket  = "my-terraform-state"
    key     = "speedy/prod/terraform.tfstate"
    region  = "eu-west-1"
    encrypt = true
  }
}

provider "aws" {
  region = var.region
}

# ── Variables ────────────────────────────────────────────────────────────────

variable "region"       { default = "eu-west-1" }
variable "environment"  { default = "prod" }
variable "instance_type"{ default = "t3.small" }

# ── VPC ──────────────────────────────────────────────────────────────────────

module "vpc" {
  source  = "terraform-aws-modules/vpc/aws"
  version = "~> 5.0"

  name            = "speedy-${var.environment}"
  cidr            = "10.0.0.0/16"
  azs             = ["${var.region}a", "${var.region}b"]
  private_subnets = ["10.0.1.0/24", "10.0.2.0/24"]
  public_subnets  = ["10.0.101.0/24", "10.0.102.0/24"]
  enable_nat_gateway = true
}

# ── ECS cluster ──────────────────────────────────────────────────────────────

resource "aws_ecs_cluster" "main" {
  name = "speedy-${var.environment}"

  setting {
    name  = "containerInsights"
    value = "enabled"
  }
}

resource "aws_ecs_task_definition" "speedy" {
  family                   = "speedy"
  requires_compatibilities = ["FARGATE"]
  network_mode             = "awsvpc"
  cpu                      = "512"
  memory                   = "1024"

  container_definitions = jsonencode([{
    name      = "speedy"
    image     = "${aws_ecr_repository.speedy.repository_url}:latest"
    essential = true
    portMappings = [{ containerPort = 8080, protocol = "tcp" }]
    environment = [
      { name = "SPEEDY_LOG",   value = "info" },
      { name = "SPEEDY_MODEL", value = "all-minilm:l6-v2" }
    ]
    logConfiguration = {
      logDriver = "awslogs"
      options = {
        "awslogs-group"         = "/ecs/speedy"
        "awslogs-region"        = var.region
        "awslogs-stream-prefix" = "ecs"
      }
    }
  }])
}

# ── ECR ──────────────────────────────────────────────────────────────────────

resource "aws_ecr_repository" "speedy" {
  name                 = "speedy"
  image_tag_mutability = "MUTABLE"
  image_scanning_configuration { scan_on_push = true }
}
